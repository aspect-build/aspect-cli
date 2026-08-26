//! The claim contract for `args.passthrough()` buckets.
//!
//! A bucket collects the flags the CLI could not attribute to a declared arg
//! instead of failing the parse. That trade is only sound if the task acts on
//! them, so reading a bucket means claiming it (`ctx.args.claim(name)`) and the
//! runtime verifies the claim rather than assuming it. A bucket left holding
//! flags means the user typed something that changed nothing, which is worse
//! than refusing it outright.
//!
//! Two moments enforce it, both using [`unclaimed`]:
//!
//! * Before the task reaches the tool it wraps — the earliest point at which
//!   ignoring the flags would cost real work, or reach the wrong server. Every
//!   `ctx.bazel` call that talks to Bazel checks here, so a dropped flag fails
//!   in milliseconds rather than after a build.
//! * After the task returns, as the backstop for a task that spawns nothing.
//!
//! A task that concludes without running anything can claim to say the omission
//! was deliberate (`bzl.flags.disclaim_passthrough`).

use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::ListRef;

use crate::engine::arguments::Arguments;
use crate::engine::task::TaskLike;

/// Exit code for a task that left collected flags unclaimed. Matches the CLI's
/// usage-error code: the invocation named flags that went nowhere.
pub const EXIT_UNCLAIMED: u8 = 2;

/// The names of a task's `args.passthrough()` args — its buckets. These are arg
/// names, not flag names: what a bucket *holds* is flags.
pub fn bucket_names(task: &dyn TaskLike<'_>) -> Vec<String> {
    task.args()
        .iter()
        .filter(|(_, arg)| arg.passthrough_position().is_some())
        .map(|(name, _)| name.clone())
        .collect()
}

/// The buckets among `buckets` that hold flags nobody claimed, as
/// `(bucket name, flags)`. Empty when every bucket is either claimed or empty —
/// the ordinary case, since the built-in Bazel flag helpers claim as they
/// resolve.
pub fn unclaimed<'v>(buckets: &[String], args: Value<'v>) -> Vec<(String, Vec<String>)> {
    let Some(store) = args.downcast_ref::<Arguments>() else {
        return vec![];
    };
    buckets
        .iter()
        .filter(|name| !store.is_claimed_key(name))
        .filter_map(|name| {
            let flags: Vec<String> = store
                .get(name)
                .and_then(ListRef::from_value)
                .map(|list| list.iter().map(|flag| flag.to_str()).collect())
                .unwrap_or_default();
            (!flags.is_empty()).then(|| (name.clone(), flags))
        })
        .collect()
}

/// What the user is told when their flags went nowhere: which flags, which task
/// dropped them, and the two ways to resolve it.
///
/// `about_to` names what the task was on the verge of doing, so the message
/// reads for both enforcement points ("run bazel", "return").
pub fn message(task_kind: &str, about_to: &str, bucket: &str, flags: &[String]) -> String {
    let (flag_noun, them) = if flags.len() == 1 {
        ("flag", "it")
    } else {
        ("flags", "them")
    };
    format!(
        "the {flag_noun} {} would have no effect: {task_kind} collected {them} into its \
         `{bucket}` passthrough arg and never claimed {them} (`ctx.args.claim(\"{bucket}\")`), \
         so nothing forwarded {them} anywhere before it was about to {about_to}. Either \
         forward the {flag_noun} or claim the arg to say this run deliberately ignores {them}.",
        flags.join(" "),
    )
}

/// Refuse to proceed while any bucket in `buckets` holds unclaimed flags.
///
/// For the pre-execution check: the returned error carries the same explanation
/// the post-run report gives, so the two read alike wherever the user meets them.
pub fn require_claimed<'v>(
    task_kind: &str,
    about_to: &str,
    buckets: &[String],
    args: Value<'v>,
) -> anyhow::Result<()> {
    match unclaimed(buckets, args).first() {
        None => Ok(()),
        Some((bucket, flags)) => Err(anyhow::anyhow!(message(task_kind, about_to, bucket, flags))),
    }
}

#[cfg(test)]
mod tests {
    use super::{message, unclaimed};
    use crate::engine::arguments::Arguments;
    use starlark::values::Heap;
    use starlark::values::ValueLike;
    use starlark::values::list::AllocList;

    /// The core the two enforcement points share: only a bucket that holds
    /// something and was never claimed is reported.
    #[test]
    fn only_unclaimed_non_empty_buckets_are_reported() {
        let buckets = ["held".to_owned(), "claimed".to_owned(), "empty".to_owned()];
        Heap::temp(|heap| {
            let store = Arguments::new();
            store.insert("held".to_owned(), heap.alloc(AllocList(["--jobs", "8"])));
            store.insert(
                "claimed".to_owned(),
                heap.alloc(AllocList(["--keep_going"])),
            );
            store.insert(
                "empty".to_owned(),
                heap.alloc(AllocList(Vec::<&str>::new())),
            );
            let args = heap.alloc(store);

            let store = args
                .downcast_ref::<Arguments>()
                .expect("the store round-trips through the heap");
            store.claim("claimed");

            assert_eq!(
                unclaimed(&buckets, args),
                vec![("held".to_owned(), vec!["--jobs".to_owned(), "8".to_owned()])]
            );
            store.claim("held");
            assert!(unclaimed(&buckets, args).is_empty());
        });
    }

    /// A value that is not a list cannot hold routed flags, and a store the
    /// runtime did not build cannot be inspected — neither is an error.
    #[test]
    fn a_non_list_or_foreign_value_reports_nothing() {
        let buckets = ["odd".to_owned()];
        Heap::temp(|heap| {
            let store = Arguments::new();
            store.insert("odd".to_owned(), heap.alloc("not a list"));
            assert!(unclaimed(&buckets, heap.alloc(store)).is_empty());
            assert!(unclaimed(&buckets, heap.alloc("not a store")).is_empty());
        });
    }

    /// The diagnostic has to name the flags, the bucket, and both remedies —
    /// it is the only place a task author learns the contract at runtime.
    #[test]
    fn the_message_names_the_flags_and_both_remedies() {
        let one = message("build", "return", "rest", &["--jobs=8".to_owned()]);
        assert!(
            one.contains("the flag --jobs=8 would have no effect"),
            "{one}"
        );
        assert!(one.contains("ctx.args.claim(\"rest\")"), "{one}");
        assert!(one.contains("about to return"), "{one}");

        let many = message(
            "build",
            "run bazel",
            "rest",
            &["-c".to_owned(), "opt".to_owned()],
        );
        assert!(
            many.contains("the flags -c opt would have no effect"),
            "{many}"
        );
        assert!(many.contains("about to run bazel"), "{many}");
    }
}
