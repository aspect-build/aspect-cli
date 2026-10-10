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
//! deliberately carries no handle is named in `DEVIATIONS`, which records that
//! file's whole shape — spawn sites, handles, pumps — beside the reason,
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

/// One file whose wiring is not the default shape, recorded in full.
///
/// Every count is asserted, so a file listed here cannot quietly grow another
/// spawn site or lose a handle: `sites` catches a new invocation, `opens` and
/// `pumps` catch one that stopped dispatching. An earlier version recorded only
/// `opens`, which meant a fourth unwired `ctx.bazel.build(` in `delivery.axl`
/// changed nothing it checked — the two files most likely to grow an invocation
/// were exactly where the check stopped working.
struct Wiring {
    path: &'static str,
    /// Real `ctx.bazel.build(` / `.test(` call sites in the file.
    sites: usize,
    /// `exec_log.open` calls. Fewer than `sites` means some invocation
    /// dispatches nothing, which `why` has to justify.
    opens: usize,
    /// `exec_log.pump` calls. Fewer than `opens` means a handle is drained only
    /// at the end of its build — correct for a spawn with no event loop to pump
    /// from, and a timeliness difference rather than a lost entry.
    pumps: usize,
    why: &'static str,
}

/// The default: one handle per spawn site, pumped and closed once each. A file
/// absent from [`DEVIATIONS`] is held to it.
///
/// Add a row only for an invocation that cannot or should not carry a handle —
/// never for one whose wiring has not been written yet. The reason is the
/// record, so write one that will still answer the question in a year.
const DEVIATIONS: &[Wiring] = &[
    Wiring {
        path: "bazel/invocation.axl",
        sites: 2,
        opens: 1,
        pumps: 1,
        why: "`ctx.bazel.test` and `ctx.bazel.build` are the two arms of one               branch — the handle is opened above it, per attempt, and whichever               arm runs gets it",
    },
    Wiring {
        path: "cache_diff.axl",
        sites: 3,
        opens: 1,
        pumps: 0,
        why: "two of the three spawns are the probe's invalidate and observe               passes, which run under `--experimental_remote_require_cached`: it               denies anything not already cached, so nothing executes and there               is nothing to log. The third, `--mode=precise`'s pre-pass, does               execute and upload actions and is wired. It pumps nothing because               it asks for no event stream and so has no drain loop to pump from,               which leaves every hook firing from `close`",
    },
    Wiring {
        path: "delivery.axl",
        sites: 3,
        opens: 2,
        pumps: 2,
        why: "phase 2, the checksum re-run, has no handle on purpose: it is a               cache lookup over phase 1's warm analysis under               `--experimental_remote_require_cached`, so every spawn it would log               is either a probe for an action phase 1 already logged or a               `DeliveryHash` probe that stands for no work the user asked for. A               hook auditing spawns would double-count the first and have to learn               to ignore the second. The trait's `build_event` hooks are withheld               from that phase for the same reason, and the two streams describe               the same thing",
    },
    Wiring {
        path: "private/lib/cache_selection.axl",
        sites: 1,
        opens: 0,
        pumps: 0,
        why: "a `--noanalyze` probe: it prevents analysis and every build and               test action, so Bazel runs nothing and logs nothing",
    },
    Wiring {
        path: "bazel/invocation_test.axl",
        sites: 1,
        opens: 0,
        pumps: 0,
        why: "a test fixture driving ctx.bazel.build directly, not a task",
    },
];

fn deviation(path: &str) -> Option<&'static Wiring> {
    DEVIATIONS.iter().find(|w| w.path == path)
}

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

/// Every spawn site opens a handle, unless the file says otherwise in full.
///
/// Per *site*, not per file: a file that drives Bazel several times would
/// otherwise pass on one wired call site, which is how `delivery.axl` passed
/// while two of its three phases dispatched nothing. Counting sites makes the
/// second and third invocation of a multi-phase task decisions somebody made
/// rather than call sites nobody looked at.
#[test]
fn every_spawn_site_opens_a_handle_or_is_recorded() {
    for (path, source) in sources() {
        let sites = spawn_lines(source).len();
        if sites == 0 {
            continue;
        }
        let opens = calls(source, "open").len();
        let Some(w) = deviation(&path) else {
            assert_eq!(
                opens,
                sites,
                "{path} spawns Bazel at {sites} site(s) but opens {opens} \
                 `exec_log_event` handle(s), so some invocation dispatches nothing. \
                 Wire it as `builtins/aspect/bazel/exec_log.axl` documents (open \
                 before the spawn, pump in the drain loop, close before wait()), or \
                 add a DEVIATIONS row in {} recording this file's shape and why that \
                 spawn cannot carry a handle.",
                file!(),
            );
            continue;
        };
        assert_eq!(
            sites, w.sites,
            "{path} now has {sites} Bazel spawn site(s); DEVIATIONS records {}. A new \
             invocation needs its own handle, or the row needs updating together with \
             its reason ({}).",
            w.sites, w.why,
        );
        assert_eq!(
            opens, w.opens,
            "{path} opens {opens} `exec_log_event` handle(s); DEVIATIONS records {} \
             ({}). Update the row and its reason together, or wire the remaining \
             site(s).",
            w.opens, w.why,
        );
        assert!(
            w.opens < w.sites || w.pumps < w.opens,
            "{path} is in DEVIATIONS but now matches the default shape — drop the row",
        );
    }
}

/// Every table row still describes a file that exists.
///
/// A row for a renamed or deleted file is an exemption nothing enforces and a
/// reason nobody can check, and it would silently excuse a *new* file that
/// happened to take the old path.
#[test]
fn every_recorded_deviation_matches_a_real_file() {
    let paths: Vec<String> = sources().into_iter().map(|(path, _)| path).collect();
    for w in DEVIATIONS {
        assert!(
            paths.iter().any(|p| p == w.path),
            "DEVIATIONS names {}, which is not in the built-in tree — the file was \
             renamed or removed, so drop or re-point the row",
            w.path,
        );
    }
}

/// Each opened handle is drained, and pumped as often as the file says.
///
/// `close` is not optional: without it a hook loses whatever had not arrived by
/// the end of the build. `pump` is, because a spawn with no event stream has no
/// drain loop to put one in — every hook then fires from `close`, which costs
/// timeliness and no entries. Both leave the task working, which is why they are
/// counted here rather than noticed.
#[test]
fn every_opened_handle_is_closed_and_pumped_as_recorded() {
    for (path, source) in sources() {
        let opens = calls(source, "open").len();
        if opens == 0 {
            continue;
        }
        if path.ends_with("_test.axl") || path == "bazel/exec_log.axl" {
            continue; // defines or unit-tests the verbs rather than using them
        }
        assert_eq!(
            calls(source, "close").len(),
            opens,
            "{path}: one `exec_log.close` per `exec_log.open` — a handle that is \
             never drained loses every entry still in flight when `wait()` releases it",
        );
        let expected_pumps = deviation(&path).map_or(opens, |w| w.pumps);
        assert_eq!(
            calls(source, "pump").len(),
            expected_pumps,
            "{path}: expected {expected_pumps} `exec_log.pump` call(s){}. Pump in the \
             invocation's own drain loop, or record the difference in DEVIATIONS in {}.",
            deviation(&path).map_or(String::new(), |w| format!(" ({})", w.why)),
            file!(),
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
