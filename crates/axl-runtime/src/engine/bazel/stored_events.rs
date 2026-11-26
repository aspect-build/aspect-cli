//! `bazel.build_events.read(path)`: a build event stream kept on disk, such
//! as a build's `build_events.binpb` in the task history, iterated as the
//! same `BuildEvent` values a live build's `bazel.build_events.iterator()`
//! yields, so code written for a build's events reads an earlier build's.
//! `bazel.build_events.follow(path)`: one still being written, by another
//! process's build, polled for what it has written so far.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use allocative::Allocative;
use axl_proto::build_event_stream::BuildEvent;
use bazel::bes::stored::{Follower, Stored};
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::{
    AllocValue, Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Trace, Value, ValueLike,
    starlark_value,
};

/// The events of a stream kept on disk, read as they are iterated: one pass,
/// every loop over it going on from where the last stopped.
#[derive(Clone, Display, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<bazel.build.StoredBuildEvents>")]
pub struct StoredBuildEvents {
    #[allocative(skip)]
    stored: Arc<Mutex<Stored>>,
    #[allocative(skip)]
    kinds: Option<Arc<HashSet<i32>>>,
}

impl std::fmt::Debug for StoredBuildEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredBuildEvents").finish_non_exhaustive()
    }
}

impl StoredBuildEvents {
    pub fn open(path: &Path, kinds: Option<HashSet<i32>>) -> anyhow::Result<Self> {
        let stored = Stored::open(path).map_err(|e| {
            anyhow::anyhow!("cannot read the build events at {}: {e}", path.display())
        })?;
        Ok(Self {
            stored: Arc::new(Mutex::new(stored)),
            kinds: kinds.map(Arc::new),
        })
    }
}

impl<'v> AllocValue<'v> for StoredBuildEvents {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

/// How many times `limit` events of any kind one filtered `poll()` reads
/// through for those of its kinds.
const RAW_PER_POLL: usize = 10;

#[starlark_value(type = "bazel.build.StoredBuildEvents")]
impl<'v> StarlarkValue<'v> for StoredBuildEvents {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let stored = self.stored.lock().unwrap();
        match attribute {
            "complete" => Some(heap.alloc(stored.complete())),
            "error" => Some(match stored.error() {
                Some(e) => heap.alloc(e),
                None => Value::new_none(),
            }),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "complete" | "error")
    }

    unsafe fn iterate(&self, me: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(me)
    }

    unsafe fn iter_next(&self, _i: usize, heap: Heap<'v>) -> Option<Value<'v>> {
        let mut stored = self.stored.lock().unwrap();
        for event in stored.by_ref() {
            let wanted = self
                .kinds
                .as_ref()
                .is_none_or(|kinds| bazel::bes::kind::event_kind_in(&event, kinds));
            if wanted {
                return Some(event.alloc_value(heap));
            }
        }
        None
    }

    unsafe fn iter_stop(&self) {}
}

/// A stream another process's build is still writing, polled for the whole
/// events written so far (`poll()`), never waiting.
#[derive(Clone, Display, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<bazel.build.FollowedBuildEvents>")]
pub struct FollowedBuildEvents {
    #[allocative(skip)]
    follower: Arc<Mutex<Follower>>,
    #[allocative(skip)]
    kinds: Option<Arc<HashSet<i32>>>,
    /// Events read past a restart in a poll that had already taken some
    /// from before it, held for the next poll so a restart only ever comes
    /// at a batch's start; and the restarts the batches returned so far
    /// crossed, which `restarts` reports.
    #[allocative(skip)]
    carried: Arc<Mutex<(Vec<BuildEvent>, u32)>>,
}

impl std::fmt::Debug for FollowedBuildEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FollowedBuildEvents")
            .finish_non_exhaustive()
    }
}

impl FollowedBuildEvents {
    pub fn new(path: &Path, kinds: Option<HashSet<i32>>) -> Self {
        Self {
            follower: Arc::new(Mutex::new(Follower::new(path))),
            kinds: kinds.map(Arc::new),
            carried: Default::default(),
        }
    }
}

impl<'v> AllocValue<'v> for FollowedBuildEvents {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "bazel.build.FollowedBuildEvents")]
impl<'v> StarlarkValue<'v> for FollowedBuildEvents {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("followed_methods", followed_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let follower = self.follower.lock().unwrap();

        // Events held past a restart are not returned yet: the stream is
        // neither complete nor stopped as far as `poll()` has gone.
        let held = !self.carried.lock().unwrap().0.is_empty();
        match attribute {
            "complete" => Some(heap.alloc(follower.complete() && !held)),
            "error" => Some(match follower.error() {
                Some(e) if !held => heap.alloc(e),
                _ => Value::new_none(),
            }),
            "restarts" => Some(heap.alloc(self.carried.lock().unwrap().1)),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "complete" | "error" | "restarts")
    }
}

/// What a followed stream gives `next_batch`: [`Follower`], or a test's.
trait Source {
    fn poll(&mut self, limit: usize) -> Vec<BuildEvent>;
    fn restarts(&self) -> u32;
}

impl Source for Follower {
    fn poll(&mut self, limit: usize) -> Vec<BuildEvent> {
        Follower::poll(self, limit)
    }
    fn restarts(&self) -> u32 {
        Follower::restarts(self)
    }
}

/// One `poll()`: up to `limit` events `keep` takes, read through at most
/// `RAW_PER_POLL` times it, never across a restart. Events read past one
/// once the batch has some from before it are held in `carried.0` for the
/// next calls, which start with them, `limit` at a time; `carried.1` is
/// the restarts the batches returned so far crossed.
fn next_batch(
    source: &mut impl Source,
    carried: &mut (Vec<BuildEvent>, u32),
    limit: usize,
    keep: impl Fn(&BuildEvent) -> bool,
) -> Vec<BuildEvent> {
    let mut kept = Vec::new();
    let mut read = 0;
    while kept.len() < limit && read < limit.saturating_mul(RAW_PER_POLL) {
        let (events, restarts) = if carried.0.is_empty() {
            let before = source.restarts();
            let events = source.poll(limit - kept.len());
            let after = source.restarts();
            if after != before && read > 0 {
                carried.0 = events;
                break;
            }
            (events, after)
        } else {
            let take = (limit - kept.len()).min(carried.0.len());
            (carried.0.drain(..take).collect(), source.restarts())
        };
        if events.is_empty() {
            break;
        }
        carried.1 = restarts;
        read += events.len();
        kept.extend(events.into_iter().filter(|event| keep(event)));
    }
    kept
}

#[starlark_module]
fn followed_methods(registry: &mut MethodsBuilder) {
    /// The whole events written since the last call, oldest first, at most
    /// `limit` (the rest come next time), of the `kinds=` the stream was
    /// followed for. Never waits: none when nothing new is whole yet or the
    /// file is not there yet. `restarts`, read after it, counts the restarts
    /// the calls so far crossed: when it grows, this call's events are the
    /// restarted stream's first, none from before it. Followed with
    /// `kinds=`, one call reads through
    /// at most ten times `limit` events of every kind, so over a long
    /// backlog it may return fewer, or none, with more of the file unread:
    /// poll again.
    fn poll<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = 10000)] limit: u32,
        heap: Heap<'v>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        let me = this
            .downcast_ref::<FollowedBuildEvents>()
            .ok_or_else(|| anyhow::anyhow!("not a FollowedBuildEvents"))?;
        // The limit counts the events of the kinds followed, read through at
        // most `RAW_PER_POLL` times it of every kind, so a filtered poll over
        // a long backlog returns, with none perhaps, rather than blocking.
        let limit = limit as usize;
        let mut follower = me.follower.lock().unwrap();
        let mut carried = me.carried.lock().unwrap();
        let kept = next_batch(&mut *follower, &mut carried, limit, |event| {
            me.kinds
                .as_ref()
                .is_none_or(|kinds| bazel::bes::kind::event_kind_in(event, kinds))
        });
        Ok(kept
            .into_iter()
            .map(|event| event.alloc_value(heap))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads that each give their events, then the restarts after them.
    struct Script(Vec<(Vec<BuildEvent>, u32)>, u32);

    impl Source for Script {
        fn poll(&mut self, _limit: usize) -> Vec<BuildEvent> {
            if self.0.is_empty() {
                return vec![];
            }
            let (events, restarts) = self.0.remove(0);
            self.1 = restarts;
            events
        }
        fn restarts(&self) -> u32 {
            self.1
        }
    }

    fn event(last: bool) -> BuildEvent {
        BuildEvent {
            last_message: last,
            ..Default::default()
        }
    }

    /// A restart between two reads of one call ends the batch there: what
    /// came after it starts the next, and `restarts` grows only with it.
    #[test]
    fn a_batch_never_spans_a_restart() {
        let mut source = Script(
            vec![(vec![event(false)], 0), (vec![event(true), event(true)], 1)],
            0,
        );
        let mut carried = Default::default();
        // Kept only last messages, so the first read leaves the batch short
        // and a second is made.
        let keep = |e: &BuildEvent| e.last_message;
        assert_eq!(next_batch(&mut source, &mut carried, 2, keep).len(), 0);
        assert_eq!(carried.1, 0, "no restart crossed yet");
        assert_eq!(next_batch(&mut source, &mut carried, 1, keep).len(), 1);
        assert_eq!(carried.1, 1, "the restarted stream's first batch");
        assert_eq!(carried.0.len(), 1, "held ones are given limit at a time");
        assert_eq!(next_batch(&mut source, &mut carried, 1, keep).len(), 1);
        assert!(carried.0.is_empty());
    }
}
