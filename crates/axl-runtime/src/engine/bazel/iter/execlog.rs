//! The two AXL-facing views of a build's decoded execution log.
//!
//! [`ExecutionLogIterator`] backs `build.execution_logs()`: unfiltered, and handed
//! out after the spawn. It takes the stream's *own* subscriber, which is in the
//! ring from the moment the stream spawns, so the reader blocks for it like any
//! other consumer and the iteration is complete.
//!
//! [`ExecLogIter`] backs `bazel.execution_log.iterator(kinds = [...])`: created
//! before the spawn and passed in `execution_log = [...]`. Both forms are
//! complete; what the handle adds is its `kinds=` filter, applied here in Rust so
//! a consumer that only wants spawns does not pay a Starlark allocation per file
//! in the build, and a subscriber of its own, so it can sit beside a decoded
//! `file()` sink where `build.execution_logs()` cannot.
//!
//! Both hold a subscriber the producer blocks for, so both have to be releasable
//! from outside: `build.wait()` releases every one still held before joining the
//! reader, which is what bounds a consumer that stopped reading. That lifecycle,
//! and the cancellation-aware blocking read it wraps, live once in
//! [`Subscription`] — what remains here is what the two views do *not* share:
//! `kinds=` filtering and a deferred bind for the handle, nothing for the other.

use std::collections::HashSet;
use std::sync::Arc;

use allocative::Allocative;
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
use fibre::spmc::Receiver;

use crate::engine::cancellation::Signals;
use bazel::execlog::subscription::Subscription;

/// The `build.execution_logs()` iterator: the stream's own subscriber, handed to
/// AXL as an iterable.
///
/// Nothing but a [`Subscription`] and the signals a blocking read needs — this
/// view adds no filter and no deferred bind, so the shared lifecycle is the whole
/// of it.
#[derive(Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative, Debug)]
#[display("<execlog_iterator>")]
pub struct ExecutionLogIterator {
    #[allocative(skip)]
    #[trace(unsafe_ignore)]
    sub: Subscription,
    /// The run's cancellation state, so blocking iteration stays a safe point —
    /// as for [`ExecLogIter::signals`]. See `iter_next`.
    #[allocative(skip)]
    signals: Arc<Signals>,
}

impl ExecutionLogIterator {
    pub fn new(signals: Arc<Signals>, recv: Receiver<ExecLogEntry>) -> Self {
        Self {
            sub: Subscription::live(recv),
            signals,
        }
    }

    /// Unsubscribe, dropping any entries still buffered. Idempotent.
    ///
    /// `build.wait()` calls this before joining the stream, for the same reason
    /// it releases every [`ExecLogIter`]: the subscriber this holds is one the
    /// producer blocks for, so an iteration that stopped early — or never
    /// started — would otherwise park the reader on a consumer that is not
    /// coming. The stream cannot do it itself; `execution_logs()` took the
    /// receiver out of it.
    pub fn release(&self) {
        self.sub.release();
    }
}

impl<'v> AllocValue<'v> for ExecutionLogIterator {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_module]
pub(crate) fn execlog_methods(registry: &mut MethodsBuilder) {
    /// Non-blocking pop of the next entry, or `None` when nothing is buffered
    /// yet. The producer runs up to the channel capacity ahead and then waits
    /// for this subscriber, so polling slowly costs time rather than entries.
    fn try_pop<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<ExecLogEntry>> {
        let this = this
            .downcast_ref_err::<ExecutionLogIterator>()
            .into_anyhow_result()?;
        Ok(NoneOr::from_option(this.sub.try_pop(|_| true)))
    }

    /// Returns `True` once the stream has ended and every entry has been
    /// received via `for` or repeated `try_pop`, or the subscriber has been
    /// released (`build.wait()`).
    fn done<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let this = this
            .downcast_ref_err::<ExecutionLogIterator>()
            .into_anyhow_result()?;
        Ok(this.sub.is_drained())
    }

    /// Stop consuming: unsubscribe and drop whatever is buffered. Idempotent.
    /// As on an [`ExecLogIter`] handle — a task that decides mid-build it no
    /// longer wants entries releases the producer's blocking-send path rather
    /// than holding the end-of-build join open until `wait()` does it.
    fn drain<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let this = this
            .downcast_ref_err::<ExecutionLogIterator>()
            .into_anyhow_result()?;
        this.release();
        Ok(NoneType)
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
    /// Blocks until the next entry or the end of the stream. Unfiltered: every
    /// entry yields. See [`Subscription::recv_blocking`] for why it is
    /// cancellation-aware.
    unsafe fn iter_next(&self, _index: usize, heap: Heap<'v>) -> Option<values::Value<'v>> {
        self.sub
            .recv_blocking(|| self.signals.should_unwind(), |_| true)
            .map(|entry| entry.alloc_value(heap))
    }

    /// Leaving a `for` loop is not unsubscribing: the subscriber stays live, so
    /// `try_pop` and a second loop resume where the first stopped rather than
    /// starting over. Only `build.wait()` releases it. See
    /// [`ExecutionLogIterator::release`].
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

/// A `bazel.execution_log.iterator()` handle: a kinds-filtered subscriber to
/// one build's decoded execution log, created before the spawn and passed in
/// `execution_log = [...]`.
///
/// Why a handle rather than `build.execution_logs()`: cost and composition, not
/// completeness. Both forms receive every entry — the producer blocks for
/// whatever subscribers it has, see
/// [`ExecLogStream::spawn_with_file`](bazel::execlog::feed::ExecLogStream::spawn_with_file).
/// A handle clones a subscriber of its own, so it combines with sinks where
/// `execution_logs()` takes the stream's one and only, and it filters in Rust,
/// below.
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
    /// Starts [`Subscription::pending`]: this handle exists before the build it
    /// will be bound to.
    #[allocative(skip)]
    #[trace(unsafe_ignore)]
    sub: Subscription,
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
            sub: Subscription::pending(),
            signals,
        }
    }

    /// Subscribe this handle to a build's decoded execution log. Called by
    /// `Build::spawn` with a clone of the stream's receiver; one handle binds
    /// once, so a handle reused across two builds is an error rather than a
    /// stream silently attached to the wrong one.
    pub fn bind(&self, recv: Receiver<ExecLogEntry>) -> anyhow::Result<()> {
        if self.sub.bind(recv) {
            return Ok(());
        }
        anyhow::bail!(
            "this `bazel.execution_log.iterator()` handle was already bound to a build; \
             create a fresh one per build",
        )
    }

    /// Unsubscribe, dropping any entries still buffered.
    ///
    /// `build.wait()` calls this before joining the stream, which is what bounds
    /// a task that never drained its handle: the producer blocks for every
    /// subscriber, so the reader would otherwise wait on a consumer that is not
    /// coming. The AXL side drains first (`bazel/exec_log.axl`) and so loses
    /// nothing to it.
    pub fn release(&self) {
        self.sub.release();
    }

    /// Whether this handle still holds a subscriber.
    ///
    /// For the all-or-nothing bind in `build::bind_execlog_iters`, whose
    /// rollback nothing else can observe: `release` and a drained handle both
    /// end in `Done`, so a later `bind` fails whether or not the rollback ran.
    #[cfg(test)]
    pub fn is_live(&self) -> bool {
        self.sub.is_live()
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

    /// `done` here means "this handle no longer holds a subscriber" — released by
    /// `drain()`, a cancel, or `build.wait()`. Narrower than
    /// `ExecutionLogIterator.done()`, which answers "nothing more can arrive" and
    /// is also true of a still-subscribed handle whose stream has closed.
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<values::Value<'v>> {
        match attribute {
            "done" => Some(heap.alloc(self.sub.is_done())),
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

    /// Blocks until the next entry this handle's `kinds=` filter wants, or the end
    /// of the stream. Entries of other kinds are discarded rather than yielded as
    /// `None`, unlike the BES iterator's quiet tick. See
    /// [`Subscription::recv_blocking`] for why it is cancellation-aware.
    unsafe fn iter_next(&self, _index: usize, heap: Heap<'v>) -> Option<values::Value<'v>> {
        self.sub
            .recv_blocking(|| self.signals.should_unwind(), |entry| self.wants(entry))
            .map(|entry| entry.alloc_value(heap))
    }

    /// Leaving a `for` loop is not unsubscribing: the subscriber stays live, so
    /// `try_pop` and a second loop resume where the first stopped rather than
    /// starting over. Only `build.wait()` releases it. See
    /// [`ExecLogIter::release`].
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
        Ok(NoneOr::from_option(iter.sub.try_pop(|e| iter.wants(e))))
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

    /// After `release()` the handle behaves as a drained one, and — the part that
    /// matters — its subscriber is gone, so a reader blocked on it is freed.
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
        // parking forever, which is what `bazel::execlog::feed` reads as end-of-stream.
        let err = sender.send(file(2));
        assert!(
            err.is_err(),
            "expected Closed once the only subscriber was released"
        );
    }

    /// Blocking iteration must stay a safe point: with no end-of-stream in sight,
    /// a cancelled run has to end the loop rather than wait in it.
    ///
    /// Timed out because the regression is a park, not a wrong answer: the
    /// channel below is never written to and never closed, so an `iter_next` that
    /// missed the cancel waits forever and reports nothing.
    #[test]
    fn iteration_ends_on_a_cancel_rather_than_parking() {
        let outcome = crate::test::with_timeout(std::time::Duration::from_secs(10), || {
            let signals = Signals::new();
            // A sender that is never written to and never closed: iteration has
            // nothing to return and no end-of-stream to end on.
            let (_sender, recv) = bounded::<ExecLogEntry>(4);
            let iter = ExecLogIter::new(signals.clone(), None);
            iter.bind(recv).unwrap();

            signals.force();
            assert!(signals.should_unwind(), "the run should now be unwinding");

            let ended = Heap::temp(|heap| unsafe { iter.iter_next(0, heap) }.is_none());
            // Ending also releases the subscriber, which is what frees a producer
            // already parked on it.
            (ended, iter.sub.is_done())
        });
        let Some((ended, unsubscribed)) = outcome else {
            panic!(
                "timed out: a cancelled run parked in `iter_next` instead of ending the iteration"
            )
        };
        assert!(
            ended,
            "a cancelled run must end the iteration instead of parking in it",
        );
        assert!(
            unsubscribed,
            "a cancelled iteration should leave the handle unsubscribed",
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
        iter.sub.try_pop(|e| iter.wants(e))
    }

    /// The AXL-visible half of [`Subscription`]'s on-loan guarantee: a handle
    /// mid-read is still a bound handle, so `bind` refuses it. See
    /// `subscription::State::Reading` for what conflating the two would cost,
    /// and why this is asserted though AXL cannot reach it.
    #[test]
    fn a_handle_mid_read_cannot_be_rebound() {
        let (_s1, r1) = bounded::<ExecLogEntry>(4);
        let (_s2, r2) = bounded::<ExecLogEntry>(4);
        let iter = ExecLogIter::new(Signals::new(), None);
        iter.bind(r1).unwrap();

        let on_loan = iter.sub.take_for_read().expect("a bound handle lends out");
        let err = iter
            .bind(r2)
            .expect_err("a bind mid-read must be refused")
            .to_string();
        assert!(err.contains("already bound"), "unexpected error: {err}");

        iter.sub.finish_read(on_loan);
        assert!(iter.is_live(), "the read should have put its receiver back");
    }

    /// Pins the AXL shape of `ExecLogEntry.Output`'s oneof, which the resolver in
    /// `@aspect//private/lib/execlog.axl` reads to build a spawn's identity: an
    /// output is either an id into the entry table (`type(...) == "int"`) or the
    /// raw path Bazel recorded when it could not resolve one (`"string"`).
    ///
    /// Worth a test of its own because a mismatch degrades silently rather than
    /// failing: `_spawn_key` would fall back to `label!mnemonic` for every spawn,
    /// colliding siblings, and `outputs()` would come back empty. The entry is a
    /// real decoded proto rather than a hand-built struct, so a change in how
    /// starbuf represents a scalar oneof breaks this test and not the resolver.
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
