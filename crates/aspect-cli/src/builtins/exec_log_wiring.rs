//! Every built-in task that spawns Bazel wires `BazelTrait.exec_log_event`,
//! and drains it before the `wait()` that would release its handle.
//!
//! `tests/exec_log_event.rs` proves the wiring works by running each task
//! against a fake Bazel and asserting on what the hook received, which is the
//! stronger test — but it can only cover the call sites it knows about. This one
//! reads the AXL sources and so covers the call sites that exist, which is what
//! turns "a new task forgot the wiring" from a silent omission into a failure.
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

/// Files that spawn Bazel without an `exec_log_event` handle, and why.
///
/// Add to this only for an invocation that cannot produce an execution log —
/// not for one whose wiring has not been written yet.
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
