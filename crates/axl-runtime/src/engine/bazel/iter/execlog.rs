//! The two AXL-facing views of a build's decoded execution log.
//!
//! [`ExecutionLogIterator`] is the original one, handed out by
//! `build.execution_logs()` after the spawn: unfiltered, and counted too late
//! for the producer to know a consumer exists, so entries may have been dropped
//! before the first `try_pop`.
//!
//! [`ExecLogIter`] is the handle form, `bazel.execution_log.iterator(kinds =
//! [...])`, created before the spawn and passed in `execution_log = [...]`.
//! Being visible at spawn time is what makes it lossless, and its `kinds=`
//! filter is applied here in Rust so a consumer that only wants spawns does not
//! pay a Starlark allocation per file in the build.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;

use allocative::Allocative;
use fibre::TryRecvError;
use starlark::StarlarkResultExt;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::typing::Ty;
use starlark::values;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::UnpackValue;
use starlark::values::ValueLike;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;

use axl_proto::tools::protos::ExecLogEntry;
use axl_proto::tools::protos::exec_log_entry;
use derive_more::Display;
use fibre::RecvError;
use fibre::spmc::Receiver;

use crate::engine::cancellation::Signals;
use crate::engine::children::{Recv, recv_cancellable};

#[derive(ProvidesStaticType, Display, Trace, NoSerialize, Allocative, Debug)]
#[display("<execlog_iterator>")]
pub struct ExecutionLogIterator {
    #[allocative(skip)]
    recv: RefCell<Receiver<ExecLogEntry>>,
}

impl ExecutionLogIterator {
    pub fn new(recv: Receiver<ExecLogEntry>) -> Self {
        Self {
            recv: RefCell::new(recv),
        }
    }
}

impl<'v> AllocValue<'v> for ExecutionLogIterator {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_module]
pub(crate) fn execlog_methods(registry: &mut MethodsBuilder) {
    /// Returns `ExecLogEntry` if event buffer is not empty.
    /// Maximum `1000` events is buffered at once.
    fn try_pop<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<ExecLogEntry>> {
        let this = this
            .downcast_ref_err::<ExecutionLogIterator>()
            .into_anyhow_result()?;
        match this.recv.borrow_mut().try_recv() {
            Ok(it) => Ok(NoneOr::Other(it)),
            Err(TryRecvError::Empty) => Ok(NoneOr::None),
            Err(TryRecvError::Disconnected) => Ok(NoneOr::None),
        }
    }

    /// Returns `True` if stream is complete and all the events are received via `for`
    /// or calling `try_pop` repeatedly.
    fn done<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let this = this
            .downcast_ref_err::<ExecutionLogIterator>()
            .into_anyhow_result()?;
        Ok(this.recv.borrow().is_closed())
    }
}

#[starlark_value(type = "ExecutionLogIterator")]
impl<'v> values::StarlarkValue<'v> for ExecutionLogIterator {
    fn eval_type(&self) -> Option<Ty> {
        Some(Ty::iter(ExecLogEntry::get_type_starlark_repr()))
    }

    fn get_type_starlark_repr() -> Ty {
        Ty::iter(ExecLogEntry::get_type_starlark_repr())
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("execlog_methods", execlog_methods);
        Some(RES.methods())
    }

    unsafe fn iterate(
        &self,
        me: values::Value<'v>,
        _heap: Heap<'v>,
    ) -> starlark::Result<values::Value<'v>> {
        Ok(me)
    }
    unsafe fn iter_next(&self, _index: usize, heap: Heap<'v>) -> Option<values::Value<'v>> {
        // Blocks until the next entry or the stream's end. A cancelled run
        // does not end the iterator: the client is being stopped, and the
        // stream closes with it.
        match self.recv.borrow_mut().recv() {
            Ok(ev) => Some(ev.alloc_value(heap)),
            Err(RecvError::Disconnected) => None,
        }
    }
    unsafe fn iter_stop(&self) {}
}

/// Proto field number of each `ExecLogEntry.type` variant — the tag a `kinds=`
/// filter matches on. Mirrors `payload_discriminant` for build events: an
/// integer set lookup, so filtering costs nothing per entry beyond the hash.
pub fn entry_kind_tag(t: &exec_log_entry::Type) -> i32 {
    use exec_log_entry::Type;
    match t {
        Type::Invocation(_) => 2,
        Type::File(_) => 3,
        Type::Directory(_) => 4,
        Type::UnresolvedSymlink(_) => 5,
        Type::InputSet(_) => 6,
        Type::Spawn(_) => 7,
        Type::SymlinkAction(_) => 8,
        Type::SymlinkEntrySet(_) => 9,
        Type::RunfilesTree(_) => 10,
    }
}

/// True when `entry`'s payload kind is in `kinds`. A payload-less entry never
/// matches a filter — there is no kind to test — exactly as a payload-less
/// build event never matches `build_events.iterator(kinds = ...)`.
pub fn entry_kind_in(entry: &ExecLogEntry, kinds: &HashSet<i32>) -> bool {
    match entry.r#type.as_ref() {
        Some(t) => kinds.contains(&entry_kind_tag(t)),
        None => false,
    }
}

/// Resolve one `kinds=` list element to a payload tag. Accepts the raw proto
/// field number or the lower-snake-case name of the variant, which is also what
/// `type(entry.type)` returns in AXL — so a filter reads the same as the `kind
/// == "spawn"` test it replaces.
pub fn parse_entry_kind(value: values::Value) -> anyhow::Result<i32> {
    if let Some(n) = value.unpack_i32() {
        return Ok(n);
    }
    if let Some(s) = value.unpack_str() {
        return match s {
            "invocation" => Ok(2),
            "file" => Ok(3),
            "directory" => Ok(4),
            "unresolved_symlink" => Ok(5),
            "input_set" => Ok(6),
            "spawn" => Ok(7),
            "symlink_action" => Ok(8),
            "symlink_entry_set" => Ok(9),
            "runfiles_tree" => Ok(10),
            other => anyhow::bail!(
                "unknown execution log entry kind '{other}'; pass one of \
                 invocation, file, directory, unresolved_symlink, input_set, spawn, \
                 symlink_action, symlink_entry_set, runfiles_tree",
            ),
        };
    }
    anyhow::bail!(
        "kinds entry must be an `ExecLogEntry.type` variant name or its proto field number; got {}",
        value.get_type()
    )
}

/// Lifecycle of an [`ExecLogIter`]. `Live` owns the subscriber clone whose
/// existence forces the producer's blocking-send path, so leaving this state is
/// what releases the producer.
#[derive(Debug)]
enum ExecLogIterState {
    /// Created but not yet passed to a build.
    Pending,
    /// `Build::spawn` subscribed us; iteration reads from `recv`.
    Live { recv: Receiver<ExecLogEntry> },
    /// Drained, explicitly dropped, or released by `build.wait()`.
    Done,
}

/// A `bazel.execution_log.iterator()` handle: a kinds-filtered subscriber to
/// one build's decoded execution log, created before the spawn and passed in
/// `execution_log = [...]`.
///
/// Why a handle rather than `build.execution_logs()`: the decision the runtime
/// has to make before Bazel starts is whether entries may be dropped. A
/// consumer that appears only after the spawn cannot be counted, so the
/// producer would already have chosen the lossy path. Passing the handle in is
/// what lets `Build::spawn` see the consumer in time (see `lossless` on
/// [`ExecLogStream::spawn_with_file`]).
///
/// `kinds` filters by payload tag in Rust, before the entry is turned into a
/// Starlark value. That matters more here than it does for build events: a log
/// carries one entry per spawn *plus* one per file, directory, input set and
/// runfiles tree, so a hook that wants spawns alone would otherwise pay a
/// Starlark allocation and a callback for every file in the build.
#[derive(Clone, Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.execution_log.ExecLogIter>")]
pub struct ExecLogIter {
    /// `None` means no filter — every entry yields.
    #[allocative(skip)]
    kinds: Option<Arc<HashSet<i32>>>,
    #[allocative(skip)]
    state: Arc<Mutex<ExecLogIterState>>,
    /// The run's cancellation state, carried from construction so blocking
    /// iteration stays a safe point. Without it a drain would park in a bare
    /// `recv()` and the AXL body would stop answering signals — see `iter_next`.
    #[allocative(skip)]
    signals: Arc<Signals>,
}

impl ExecLogIter {
    pub fn new(signals: Arc<Signals>, kinds: Option<HashSet<i32>>) -> Self {
        Self {
            kinds: kinds.map(Arc::new),
            state: Arc::new(Mutex::new(ExecLogIterState::Pending)),
            signals,
        }
    }

    /// Subscribe this handle to a build's decoded execution log. Called by
    /// `Build::spawn` with a clone of the stream's receiver; one handle binds
    /// once, so a handle reused across two builds is an error rather than a
    /// stream silently attached to the wrong one.
    pub fn bind(&self, recv: Receiver<ExecLogEntry>) -> anyhow::Result<()> {
        let mut state = self.state.lock().unwrap();
        match *state {
            ExecLogIterState::Pending => {
                *state = ExecLogIterState::Live { recv };
                Ok(())
            }
            _ => anyhow::bail!(
                "this `bazel.execution_log.iterator()` handle was already bound to a build; \
                 create a fresh one per build",
            ),
        }
    }

    /// Unsubscribe, dropping any entries still buffered.
    ///
    /// `build.wait()` calls this before joining the execlog stream, which is
    /// what keeps a task that never drained its handle from hanging: a bound
    /// subscriber forces blocking sends, so the producer would otherwise park
    /// forever on a full channel with nobody reading. Releasing it leaves the
    /// producer with no subscribers, which it treats as "nobody is listening"
    /// and finishes. A task that drains before `wait()` — the contract the AXL
    /// side follows — loses nothing to this.
    pub fn release(&self) {
        *self.state.lock().unwrap() = ExecLogIterState::Done;
    }

    fn wants(&self, entry: &ExecLogEntry) -> bool {
        match self.kinds.as_ref() {
            Some(kinds) => entry_kind_in(entry, kinds),
            None => true,
        }
    }
}

impl<'v> AllocValue<'v> for ExecLogIter {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for ExecLogIter {
    type Error = anyhow::Error;
    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        let v = value
            .downcast_ref_err::<ExecLogIter>()
            .into_anyhow_result()?;
        Ok(Some(v.clone()))
    }
}

#[starlark_value(type = "bazel.execution_log.ExecLogIter")]
impl<'v> values::StarlarkValue<'v> for ExecLogIter {
    fn eval_type(&self) -> Option<Ty> {
        Some(Ty::iter(ExecLogEntry::get_type_starlark_repr()))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("exec_log_iter_methods", exec_log_iter_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<values::Value<'v>> {
        match attribute {
            "done" => {
                let state = self.state.lock().unwrap();
                Some(heap.alloc(matches!(*state, ExecLogIterState::Done)))
            }
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "done")
    }

    unsafe fn iterate(
        &self,
        me: values::Value<'v>,
        _heap: Heap<'v>,
    ) -> starlark::Result<values::Value<'v>> {
        Ok(me)
    }

    unsafe fn iter_next(&self, _index: usize, heap: Heap<'v>) -> Option<values::Value<'v>> {
        // Take `recv` out from under the lock so the blocking wait does not hold
        // it: `release()` has to be able to run while a drain is parked here.
        let recv = {
            let mut state = self.state.lock().unwrap();
            match std::mem::replace(&mut *state, ExecLogIterState::Pending) {
                ExecLogIterState::Live { recv } => recv,
                other => {
                    *state = other;
                    return None;
                }
            }
        };

        // Blocks until the next wanted entry or the stream's end, through
        // `recv_cancellable` rather than a bare `recv()` so the wait stays a safe
        // point: it rechecks the root token every poll slice and gives up on a
        // cancel. That is load-bearing now that an AXL drain runs here on the
        // critical path of every bazel-driving task, immediately before the one
        // other cancellation-aware wait (`wait_child`). A bare `recv()` would
        // park for as long as the reader thread does — and the reader's
        // end-of-stream signal is the daemon closing the log file, which an
        // interrupted invocation need not make happen promptly — leaving the body
        // unable to answer a second Ctrl-C or the signal deadline.
        //
        // Entries of other kinds are discarded here rather than yielded as
        // `None`: unlike the BES iterator's quiet tick, a filtered-out entry
        // carries no signal a caller could act on, and a log is mostly
        // filtered-out entries.
        loop {
            match recv_cancellable(&self.signals, &recv, None) {
                Recv::Item(entry) => {
                    if !self.wants(&entry) {
                        continue;
                    }
                    // A `release()` that landed while we were parked wins: the
                    // state is already Done and must stay that way.
                    let mut state = self.state.lock().unwrap();
                    if matches!(*state, ExecLogIterState::Pending) {
                        *state = ExecLogIterState::Live { recv };
                    }
                    return Some(entry.alloc_value(heap));
                }
                // Ending on a cancel also drops `recv`, which frees a producer
                // parked on this subscriber. The loop's next call raises the
                // task's exit; an iterator cannot raise one itself.
                Recv::Closed | Recv::Cancelled => {
                    *self.state.lock().unwrap() = ExecLogIterState::Done;
                    return None;
                }
                // Unreachable with `tick = None`, which has no deadline to pass.
                Recv::Tick => continue,
            }
        }
    }

    unsafe fn iter_stop(&self) {}
}

#[starlark_module]
pub(crate) fn exec_log_iter_methods(registry: &mut MethodsBuilder) {
    /// Non-blocking pop of the next entry this handle's `kinds=` filter wants,
    /// or `None` when nothing is buffered yet. Entries of other kinds are
    /// skipped without being seen from AXL.
    fn try_pop<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<ExecLogEntry>> {
        let iter = this
            .downcast_ref_err::<ExecLogIter>()
            .into_anyhow_result()?;
        let mut state = iter.state.lock().unwrap();
        let recv = match &*state {
            ExecLogIterState::Live { recv } => recv,
            _ => return Ok(NoneOr::None),
        };
        loop {
            match recv.try_recv() {
                Ok(entry) => {
                    if iter.wants(&entry) {
                        return Ok(NoneOr::Other(entry));
                    }
                }
                Err(TryRecvError::Empty) => return Ok(NoneOr::None),
                Err(TryRecvError::Disconnected) => {
                    *state = ExecLogIterState::Done;
                    return Ok(NoneOr::None);
                }
            }
        }
    }

    /// Stop consuming: unsubscribe and drop whatever is buffered. Idempotent.
    /// Releases the producer's blocking-send path for this subscriber, so a task
    /// that decides mid-build it no longer wants entries stops holding the
    /// end-of-build join open.
    fn drain<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let iter = this
            .downcast_ref_err::<ExecLogIter>()
            .into_anyhow_result()?;
        iter.release();
        Ok(NoneType)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fibre::spmc::bounded;
    use starlark::values::StarlarkValue;

    fn entry(id: u32, t: exec_log_entry::Type) -> ExecLogEntry {
        ExecLogEntry {
            id,
            r#type: Some(t),
        }
    }

    fn file(id: u32) -> ExecLogEntry {
        entry(
            id,
            exec_log_entry::Type::File(exec_log_entry::File {
                path: format!("f{id}.txt"),
                digest: None,
            }),
        )
    }

    fn spawn(id: u32, label: &str) -> ExecLogEntry {
        entry(
            id,
            exec_log_entry::Type::Spawn(exec_log_entry::Spawn {
                target_label: label.to_string(),
                ..Default::default()
            }),
        )
    }

    /// The tags are the compact log's proto field numbers, which the `kinds=`
    /// aliases hard-code. A variant reordered upstream would silently re-point
    /// every filter, so pin all nine.
    #[test]
    fn kind_tags_match_the_proto_field_numbers() {
        use exec_log_entry::Type;
        let cases: Vec<(Type, i32)> = vec![
            (Type::Invocation(Default::default()), 2),
            (Type::File(Default::default()), 3),
            (Type::Directory(Default::default()), 4),
            (Type::UnresolvedSymlink(Default::default()), 5),
            (Type::InputSet(Default::default()), 6),
            (Type::Spawn(Default::default()), 7),
            (Type::SymlinkAction(Default::default()), 8),
            (Type::SymlinkEntrySet(Default::default()), 9),
            (Type::RunfilesTree(Default::default()), 10),
        ];
        for (t, want) in cases {
            assert_eq!(entry_kind_tag(&t), want, "tag for {t:?}");
        }
    }

    #[test]
    fn every_alias_resolves_to_its_tag() {
        Heap::temp(|heap| {
            for (name, want) in [
                ("invocation", 2),
                ("file", 3),
                ("directory", 4),
                ("unresolved_symlink", 5),
                ("input_set", 6),
                ("spawn", 7),
                ("symlink_action", 8),
                ("symlink_entry_set", 9),
                ("runfiles_tree", 10),
            ] {
                let v = heap.alloc_str(name).to_value();
                assert_eq!(parse_entry_kind(v).unwrap(), want, "alias {name}");
            }
        });
    }

    #[test]
    fn a_raw_tag_number_passes_through() {
        Heap::temp(|heap| {
            assert_eq!(parse_entry_kind(heap.alloc(7i32)).unwrap(), 7);
        });
    }

    #[test]
    fn an_unknown_alias_names_itself_in_the_error() {
        Heap::temp(|heap| {
            let v = heap.alloc_str("spwan").to_value();
            let err = parse_entry_kind(v)
                .expect_err("expected a rejection")
                .to_string();
            assert!(err.contains("spwan"), "unexpected error: {err}");
            assert!(
                err.contains("spawn"),
                "the error should list the real kinds: {err}"
            );
        });
    }

    /// A filter tests a payload kind, so an entry with no payload has nothing to
    /// match — the same rule `event_kind_in` applies to build events.
    #[test]
    fn a_payload_less_entry_never_matches_a_filter() {
        let kinds: HashSet<i32> = [3, 7].into_iter().collect();
        let bare = ExecLogEntry {
            id: 1,
            r#type: None,
        };
        assert!(!entry_kind_in(&bare, &kinds));
        assert!(entry_kind_in(&file(2), &kinds));
    }

    /// The point of the filter: a spawns-only consumer must not be handed the
    /// file entries that make up the bulk of a real log.
    #[test]
    fn try_pop_skips_entries_of_other_kinds() {
        let (sender, recv) = bounded::<ExecLogEntry>(16);
        let iter = ExecLogIter::new(Signals::new(), Some([7].into_iter().collect()));
        iter.bind(recv).unwrap();

        for e in [
            file(1),
            file(2),
            spawn(3, "//a:a"),
            file(4),
            spawn(5, "//b:b"),
        ] {
            sender.send(e).unwrap();
        }

        let mut labels = vec![];
        while let Some(e) = pop(&iter) {
            match e.r#type {
                Some(exec_log_entry::Type::Spawn(s)) => labels.push(s.target_label),
                other => panic!("filter let through {other:?}"),
            }
        }
        assert_eq!(labels, vec!["//a:a".to_string(), "//b:b".to_string()]);
    }

    #[test]
    fn no_filter_yields_every_kind() {
        let (sender, recv) = bounded::<ExecLogEntry>(16);
        let iter = ExecLogIter::new(Signals::new(), None);
        iter.bind(recv).unwrap();
        for e in [file(1), spawn(2, "//a:a")] {
            sender.send(e).unwrap();
        }
        let mut ids = vec![];
        while let Some(e) = pop(&iter) {
            ids.push(e.id);
        }
        assert_eq!(ids, vec![1, 2]);
    }

    /// `release()` is the safety net `build.wait()` pulls. After it, the handle
    /// behaves as a drained one and — the part that matters — its subscriber
    /// clone is gone, so a producer parked on a full channel is freed.
    #[test]
    fn release_unsubscribes_and_frees_a_parked_producer() {
        // Capacity 1 so the second send parks unless the receiver is gone.
        let (sender, recv) = bounded::<ExecLogEntry>(1);
        let iter = ExecLogIter::new(Signals::new(), None);
        iter.bind(recv).unwrap();
        sender.send(file(1)).unwrap();

        iter.release();
        assert!(pop(&iter).is_none(), "a released handle yields nothing");

        // With no subscribers left the blocking send reports Closed rather than
        // parking forever, which is what `stream::execlog` reads as end-of-stream.
        let err = sender.send(file(2));
        assert!(
            err.is_err(),
            "expected Closed once the only subscriber was released"
        );
    }

    /// The reason the handle carries `Signals` at all. A drain runs on the
    /// critical path of every bazel-driving task, just before `build.wait()`, and
    /// the stream's end-of-input is the Bazel *daemon* closing the log file —
    /// which an interrupted invocation need not make happen promptly. A bare
    /// `recv()` here would leave the AXL body unable to answer the signal
    /// deadline or a second Ctrl-C.
    #[test]
    fn iteration_ends_on_a_cancel_rather_than_parking() {
        let signals = Signals::new();
        // A sender that is never written to and never closed: iteration has
        // nothing to return and no end-of-stream to end on.
        let (_sender, recv) = bounded::<ExecLogEntry>(4);
        let iter = ExecLogIter::new(signals.clone(), None);
        iter.bind(recv).unwrap();

        signals.force();
        assert!(signals.should_unwind(), "the run should now be unwinding");

        Heap::temp(|heap| {
            assert!(
                unsafe { iter.iter_next(0, heap) }.is_none(),
                "a cancelled run must end the iteration instead of parking in it",
            );
        });

        // Ending also released the subscriber, which is what frees a producer
        // already parked on it.
        let state = iter.state.lock().unwrap();
        assert!(
            matches!(*state, ExecLogIterState::Done),
            "a cancelled iteration should leave the handle Done, got {state:?}",
        );
    }

    #[test]
    fn a_handle_binds_to_one_build_only() {
        let (_s1, r1) = bounded::<ExecLogEntry>(4);
        let (_s2, r2) = bounded::<ExecLogEntry>(4);
        let iter = ExecLogIter::new(Signals::new(), None);
        iter.bind(r1).unwrap();
        let err = iter
            .bind(r2)
            .expect_err("a second bind must be refused")
            .to_string();
        assert!(err.contains("already bound"), "unexpected error: {err}");
    }

    /// `try_pop` through the Starlark method needs a heap; these tests only care
    /// about the filtering, so reach the same path without one.
    fn pop(iter: &ExecLogIter) -> Option<ExecLogEntry> {
        let mut state = iter.state.lock().unwrap();
        let recv = match &*state {
            ExecLogIterState::Live { recv } => recv,
            _ => return None,
        };
        loop {
            match recv.try_recv() {
                Ok(entry) => {
                    if iter.wants(&entry) {
                        return Some(entry);
                    }
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    *state = ExecLogIterState::Done;
                    return None;
                }
            }
        }
    }

    /// Pins the AXL shape of `ExecLogEntry.Output`'s oneof, which the resolver in
    /// `@aspect//private/lib/execlog.axl` reads to build a spawn's identity: an
    /// output is either an id into the entry table (`type(...) == "int"`) or the
    /// raw path Bazel recorded when it could not resolve one (`"string"`).
    ///
    /// Worth a test of its own because getting it wrong degrades silently rather
    /// than failing: `_spawn_key` would fall back to `label!mnemonic` for every
    /// spawn, colliding siblings, and `outputs()` would come back empty. The
    /// entry here is a real decoded proto rather than a hand-built struct, so a
    /// change in how starbuf represents a scalar oneof breaks this test instead
    /// of the resolver.
    #[test]
    fn a_decoded_spawn_exposes_its_outputs_as_ints_or_raw_strings() {
        use prost::Message;

        let entry = ExecLogEntry {
            id: 9,
            r#type: Some(exec_log_entry::Type::Spawn(exec_log_entry::Spawn {
                target_label: "//pkg:target".to_string(),
                mnemonic: "Genrule".to_string(),
                outputs: vec![
                    exec_log_entry::Output {
                        r#type: Some(exec_log_entry::output::Type::OutputId(5)),
                    },
                    exec_log_entry::Output {
                        r#type: Some(exec_log_entry::output::Type::InvalidOutputPath(
                            "gen/missing.txt".to_string(),
                        )),
                    },
                ],
                ..Default::default()
            })),
        };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.binpb");
        std::fs::write(&path, entry.encode_length_delimited_to_vec()).unwrap();

        let exit = crate::test::eval(&format!(
            r#"
def _impl(ctx):
    entries = bazel.build.execution_log.ExecLogEntry().parse_from_delimited(
        ctx.std.fs.open({path:?}),
    )
    seen = []
    for entry in entries:
        if type(entry.type) != "spawn":
            continue
        for output in entry.type.outputs:
            seen.append((type(output.type), output.type))
    if seen != [("int", 5), ("string", "gen/missing.txt")]:
        return 1
    return 0

Test = task(implementation = _impl)
"#,
            path = path.to_str().unwrap(),
        ))
        .run_task(0)
        .expect("run_task");

        assert_eq!(
            exit,
            Some(0),
            "an output id should reach AXL as an int and an unresolved path as a string",
        );
    }

    #[test]
    fn starlark_iterator_rejects_empty_kinds() {
        let err = crate::axl_check!(r#"bazel.execution_log.iterator(kinds = [])"#)
            .expect_err("expected validation error")
            .to_string();
        assert!(err.contains("kinds"), "unexpected error: {err}");
    }

    #[test]
    fn starlark_iterator_rejects_unknown_kind_string() {
        let err = crate::axl_check!(r#"bazel.execution_log.iterator(kinds = ["bogus"])"#)
            .expect_err("expected validation error")
            .to_string();
        assert!(err.contains("bogus"), "unexpected error: {err}");
    }

    #[test]
    fn starlark_iterator_accepts_kind_strings() {
        crate::axl_check!(r#"bazel.execution_log.iterator(kinds = ["spawn", "input_set"])"#)
            .expect("snippet should validate");
    }

    #[test]
    fn starlark_iterator_is_accepted_in_the_execution_log_list() {
        crate::axl_check!(
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator(kinds = ["spawn"])
    build = ctx.bazel.build("//...", execution_log = [entries, bazel.execution_log.compact_file(path = "x.zst")])
    for entry in entries:
        pass
    return build.wait().code
"#
        )
        .expect("snippet should validate");
    }
}
