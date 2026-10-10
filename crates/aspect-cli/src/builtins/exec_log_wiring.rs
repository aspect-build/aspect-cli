//! Every built-in task that spawns Bazel wires `BazelTrait.exec_log_event`,
//! and drains it before the `wait()` that would release its handle.
//!
//! `tests/exec_log_event.rs` proves the wiring works by running each task
//! against a fake Bazel and asserting on what the hook received, which is the
//! stronger test — but it can only cover the call sites it knows about. This one
//! reads the AXL sources and so covers the call sites that exist, which is what
//! turns "a new task forgot the wiring" from a silent omission into a failure.
//!
//! Coverage is per `ctx.bazel.build` / `.test` site, not per file, because the
//! tasks that drive Bazel more than once are exactly the ones where a missing
//! handle hides: a hook watching `aspect delivery` hears about three
//! invocations, and a per-file rule is satisfied by the first. A spawn that
//! deliberately carries no handle is named in `EXEMPT` (the whole file cannot
//! produce a log) or `PARTIALLY_WIRED` (some sites are wired and some are not),
//! with the reason — these two tables are where that decision is recorded, and
//! a stale entry fails rather than lingering.
//!
//! The source tree is embedded at compile time, the same way the CLI embeds it
//! to ship it, so these assertions need no data dependency and no filesystem.

use include_dir::{Dir, DirEntry, include_dir};

static ASPECT: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/builtins/aspect");

/// A real `ctx.bazel.build(` / `.test(` call, as this tree formats them: the
/// open paren ends the line. Prose mentions the calls inline
/// (`ctx.bazel.build(aspects = ...)`), so requiring the line to end there is
/// what separates a call site from a docstring.
fn spawn_lines(source: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let t = line.trim_end();
            t.ends_with("ctx.bazel.build(") || t.ends_with("ctx.bazel.test(")
        })
        .map(|(i, _)| i + 1)
        .collect()
}

/// Lines holding a call to `name(`, ignoring the `bzl.`/bare import spellings
/// and anything inside prose.
fn calls(source: &str, name: &str) -> Vec<usize> {
    let needle = format!("exec_log.{name}(");
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            !trimmed.starts_with('#') && trimmed.contains(&needle)
        })
        .map(|(i, _)| i + 1)
        .collect()
}

/// Lines where the invocation handle is awaited. The receiver is named `build`
/// or `handle` at every site; a sink's `wait()` or the cancel probe's is not
/// what `close` has to precede. Prose names these calls too, so the line has to
/// end with one — the docstring of the drain itself explains the ordering.
fn handle_waits(source: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let t = line.trim();
            !t.starts_with('#') && (t.ends_with("build.wait()") || t.ends_with("handle.wait()"))
        })
        .map(|(i, _)| i + 1)
        .collect()
}

/// Files whose every Bazel spawn goes unwired, and why.
///
/// Add to this only for an invocation that cannot produce an execution log —
/// not for one whose wiring has not been written yet. A file that wires some of
/// its spawns and not others belongs in `PARTIALLY_WIRED` instead.
const EXEMPT: &[(&str, &str)] = &[
    (
        "cache_diff.axl",
        "probe invocations: --noanalyze / --nobuild, so Bazel runs no actions and \
         logs nothing",
    ),
    (
        "private/lib/cache_selection.axl",
        "a --noanalyze probe, as above",
    ),
    (
        "bazel/invocation_test.axl",
        "a test fixture driving ctx.bazel.build directly, not a task",
    ),
];

/// Files that spawn Bazel more often than they open a handle, as
/// `(path, opens, why the difference is correct)`.
///
/// Every other file gets one `exec_log.open` per spawn site, which is what makes
/// a newly added invocation that forgot the wiring a failure rather than a hook
/// that silently stops hearing about a build. A file listed here has been looked
/// at; the reason is the record, so write one that will still answer the
/// question in a year. An entry whose `opens` has caught up with its spawn count
/// is stale and fails too.
const PARTIALLY_WIRED: &[(&str, usize, &str)] = &[
    (
        "bazel/invocation.axl",
        1,
        "`ctx.bazel.test` and `ctx.bazel.build` are the two arms of one branch — \
         the handle is opened above it, per attempt, and whichever arm runs gets it",
    ),
    (
        "delivery.axl",
        2,
        "phase 2, the checksum re-run, has no handle on purpose: it is a cache \
         lookup over phase 1's warm analysis under \
         `--experimental_remote_require_cached`, so every spawn it would log is \
         either a probe for an action phase 1 already logged or a `DeliveryHash` \
         probe that stands for no work the user asked for. A hook auditing spawns \
         would double-count the first and have to learn to ignore the second. The \
         trait's `build_event` hooks are withheld from that phase for the same \
         reason, and the two streams describe the same thing",
    ),
];

/// Every `.axl` in the built-in tree, as `(path, source)`.
fn sources() -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> = ASPECT
        .find("**/*.axl")
        .expect("the glob is a literal")
        .filter_map(|entry| match entry {
            DirEntry::File(f) => Some((
                f.path().to_string_lossy().into_owned(),
                f.contents_utf8().expect("AXL sources are UTF-8"),
            )),
            DirEntry::Dir(_) => None,
        })
        .collect();
    out.sort();
    assert!(
        out.len() > 50,
        "the built-in tree should have been embedded; found {} files",
        out.len(),
    );
    out
}

/// Nothing spawns Bazel without wiring the hook, except by name and with a
/// reason.
#[test]
fn every_task_that_spawns_bazel_wires_exec_log_event() {
    for (path, source) in sources() {
        if spawn_lines(source).is_empty() {
            continue;
        }
        if let Some((_, why)) = EXEMPT.iter().find(|(name, _)| *name == path) {
            assert!(
                calls(source, "open").is_empty(),
                "{path} is exempt from the exec_log_event wiring ({why}), but wires it \
                 anyway — drop the exemption",
            );
            continue;
        }
        assert!(
            !calls(source, "open").is_empty(),
            "{path} spawns Bazel but never calls `exec_log.open`, so no \
             `BazelTrait.exec_log_event` hook can fire for it. Wire it as \
             `builtins/aspect/bazel/exec_log.axl` documents (open before the spawn, \
             pump in the drain loop, close before wait()), or add the file to EXEMPT \
             in {} with the reason it cannot produce an execution log.",
            file!(),
        );
    }
}

/// Every spawn site has a handle of its own, or a reason in `PARTIALLY_WIRED`.
///
/// `every_task_that_spawns_bazel_wires_exec_log_event` is per *file*, so a file
/// that drives Bazel several times passes on one wired call site — which is how
/// `delivery.axl` passed while two of its three phases dispatched nothing. This
/// counts sites instead, so the second and third invocation of a multi-phase
/// task are decisions somebody made rather than call sites nobody looked at.
#[test]
fn every_spawn_site_opens_a_handle_or_is_named() {
    for (path, source) in sources() {
        let sites = spawn_lines(source).len();
        if sites == 0 || EXEMPT.iter().any(|(name, _)| *name == path) {
            continue;
        }
        let opens = calls(source, "open").len();
        let Some((_, expected, why)) = PARTIALLY_WIRED.iter().find(|(name, ..)| *name == path)
        else {
            assert_eq!(
                opens,
                sites,
                "{path} spawns Bazel at {sites} site(s) but opens {opens} \
                 `exec_log_event` handle(s), so some invocation dispatches nothing. Wire \
                 it as `builtins/aspect/bazel/exec_log.axl` documents (open before the \
                 spawn, pump in the drain loop, close before wait()), or name the file in \
                 PARTIALLY_WIRED in {} with the reason that spawn cannot or should not \
                 carry a handle.",
                file!(),
            );
            continue;
        };
        assert!(
            *expected < sites,
            "{path} is listed in PARTIALLY_WIRED ({why}), but now opens a handle at \
             every one of its {sites} spawn site(s) — drop the entry",
        );
        assert_eq!(
            opens, *expected,
            "{path} is listed in PARTIALLY_WIRED as opening {expected} handle(s) \
             ({why}); it opens {opens}. Update the entry's count and its reason \
             together, or wire the remaining site(s).",
        );
    }
}

/// Each opened handle is also pumped and drained.
///
/// A missing `pump` costs a hook every entry until the end of the build; a
/// missing `close` costs it whatever had not arrived by then. Both leave the
/// task working, which is why they are counted here rather than noticed.
#[test]
fn every_opened_handle_is_pumped_and_closed() {
    for (path, source) in sources() {
        let opens = calls(source, "open").len();
        if opens == 0 {
            continue;
        }
        if path.ends_with("_test.axl") || path == "bazel/exec_log.axl" {
            continue; // defines or unit-tests the verbs rather than using them
        }
        assert_eq!(
            calls(source, "pump").len(),
            opens,
            "{path}: one `exec_log.pump` per `exec_log.open`, in that invocation's \
             own drain loop",
        );
        assert_eq!(
            calls(source, "close").len(),
            opens,
            "{path}: one `exec_log.close` per `exec_log.open`",
        );
    }
}

/// How far after an `exec_log.close` its `wait()` may sit. Every site has it on
/// the next line; the slack is for a comment, not for another statement that
/// could consume the stream.
const WAIT_WINDOW: usize = 5;

/// `close` precedes the `wait()` it guards, at every site.
///
/// A bound hook handle is a subscriber the log reader blocks for, and `wait()`
/// joins that reader — so the drain has to happen first. `wait()` releases a
/// still-bound handle rather than deadlocking, which is exactly what makes the
/// wrong order survive review: the build still passes, the hook just stops
/// receiving entries partway through.
#[test]
fn every_drain_precedes_the_wait_it_guards() {
    for (path, source) in sources() {
        let closes = calls(source, "close");
        if closes.is_empty() || path.ends_with("_test.axl") {
            continue;
        }
        let waits = handle_waits(source);
        for close in closes {
            assert!(
                waits
                    .iter()
                    .any(|w| *w > close && *w <= close + WAIT_WINDOW),
                "{path}:{close}: `exec_log.close` must be immediately followed by the \
                 invocation's `wait()`. Found the handle awaited at {waits:?}.",
            );
            assert!(
                !waits
                    .iter()
                    .any(|w| *w < close && *w + WAIT_WINDOW >= close),
                "{path}:{close}: `exec_log.close` runs after the `wait()` that \
                 releases its handle, so the hook loses every entry that had not \
                 arrived yet. Move the drain above the wait.",
            );
        }
    }
}
