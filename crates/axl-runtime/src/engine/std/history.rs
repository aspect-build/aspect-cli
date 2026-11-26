//! `ctx.std.history`: the task history's records of past runs (each entry's
//! `task.json`; see `engine::task_record`), newest first (`list`, `get`),
//! how many it keeps (`census`, which reads them all once for `list` to
//! answer from), a run's kept output read back (`read_output`), a run as a
//! command to run it again (`command`, its arguments never handed back),
//! and the current run's links.
//!
//! A record is handed out as a dict, its command line redacted, with keys of
//! the reader's own: `dir`, the entry's folder; `outcome`, how it ended in a
//! word ([`outcome_of`]); each build's `build_events`, the stream its folder
//! kept, for `bazel.build_events.read`; and, for a run whose record says
//! `running`, `status` set to `abandoned` once its process is no longer
//! running (gone, or silent for a week), as after a crash. Records of a newer
//! schema are skipped.

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::list::UnpackList;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::{
    self, NoSerialize, ProvidesStaticType, StarlarkValue, Value, starlark_value,
};

use crate::engine::store::Env as RuntimeEnv;
use crate::engine::task_record::TaskRecord;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.History>")]
pub struct History;

impl History {
    pub fn new() -> Self {
        Self
    }
}

#[starlark_value(type = "std.History")]
impl<'v> StarlarkValue<'v> for History {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("history_methods", history_methods);
        Some(RES.methods())
    }
}

starlark_simple_value!(History);

/// A record as handed out: redacted, with its folder (`dir`), how it ended
/// (`outcome`, from `abandoned`, which the caller reads once per record so
/// the two agree), a `running` one whose process is gone marked `abandoned`,
/// and each build's kept stream (`build_events`) where its folder has one.
fn shown(dir: &std::path::Path, record: &TaskRecord, abandoned: bool) -> serde_json::Value {
    let mut value = serde_json::to_value(record.redacted()).unwrap_or_default();
    value["dir"] = dir.to_string_lossy().into_owned().into();
    if abandoned {
        value["status"] = "abandoned".into();
    }
    value["outcome"] = outcome_of(record, abandoned).into();
    // Each build's stream, where its folder kept one, for
    // `bazel.build_events.read`.
    if let Some(builds) = value.get_mut("bazel").and_then(|b| b.as_array_mut()) {
        for (build, kept) in builds.iter_mut().zip(&record.bazel) {
            let file = kept
                .folder
                .as_ref()
                .map(|f| dir.join(f).join(bazel::bes::binary_file::FILE_NAME))
                .filter(|f| f.is_file());
            if let Some(file) = file {
                build["build_events"] = file.to_string_lossy().into_owned().into();
            }
        }
    }
    value
}

/// Whether a record saying `running` is of a run no longer going (a crash's).
fn abandoned(dir: &std::path::Path, record: &TaskRecord) -> bool {
    let written = std::fs::metadata(dir.join(crate::engine::task_record::FILE_NAME))
        .and_then(|m| m.modified())
        .ok();
    record.status == "running"
        && record.pid != std::process::id()
        && !crate::engine::task_record::still_running(record, written)
}

/// How a run ended, in a word: `ok`, `failed` (a non-zero exit), `error` (an
/// error ended it), `cancelled`, `running` or `abandoned` (`abandoned`, read
/// by the caller).
fn outcome_of(record: &TaskRecord, abandoned: bool) -> &'static str {
    if record.status == "running" {
        return if abandoned { "abandoned" } else { "running" };
    }
    if !record.error.is_empty() {
        "error"
    } else if record.cancelled {
        "cancelled"
    } else if record.exit_code.unwrap_or(0) == 0 {
        "ok"
    } else {
        "failed"
    }
}

/// The most of a run's output `read_output` reads: its end.
const OUTPUT_READ_MAX: u64 = 16 << 20;

/// The end of `file`, from the start of the first whole line in its last
/// `max` bytes, and how many bytes came before that; all of it when that
/// fits.
fn output_tail(mut file: std::fs::File, max: u64) -> std::io::Result<(Vec<u8>, u64)> {
    use std::io::{Read, Seek, SeekFrom};
    let len = file.metadata()?.len();
    let mut bytes = vec![];
    if len <= max {
        file.read_to_end(&mut bytes)?;
        return Ok((bytes, 0));
    }
    // From the byte before, so a line that starts right at the cut is kept
    // whole: up to the first newline is the line the cut fell in.
    let from = len - max - 1;
    file.seek(SeekFrom::Start(from))?;
    file.take(max + 1).read_to_end(&mut bytes)?;
    let start = bytes
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |i| i + 1);
    bytes.drain(..start);
    Ok((bytes, from + start as u64))
}

/// A run's command line as typed (`argv`, the program first) as arguments to
/// run it again: without the program; before a `--`, without the flags
/// `drop` names (`--x` takes `--x`, `--x=…` and `--x:…`; an entry with `=`
/// only that flag exactly) and the argument after each `drop_with_value`
/// names written without `=`, with `add` in their place.
fn again_args(
    argv: &[String],
    drop: &[String],
    drop_with_value: &[String],
    add: &[String],
) -> Vec<String> {
    let args = argv.get(1..).unwrap_or_default();
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let dropped = |arg: &str| {
        drop.iter().any(|d| {
            if d.contains('=') {
                arg == d
            } else {
                arg == d || arg.starts_with(&format!("{d}=")) || arg.starts_with(&format!("{d}:"))
            }
        })
    };
    let mut out = vec![];
    let mut skip = false;
    for arg in &args[..end] {
        if std::mem::take(&mut skip) {
            continue;
        }
        if dropped(arg) {
            skip = drop_with_value.iter().any(|d| d == arg);
            continue;
        }
        out.push(arg.clone());
    }
    out.extend(add.iter().cloned());
    out.extend(args[end..].iter().cloned());
    out
}

/// The outcomes `list` filters by.
const OUTCOMES: &[&str] = &["ok", "failed", "error", "cancelled", "running", "abandoned"];

#[starlark_module]
fn history_methods(registry: &mut MethodsBuilder) {
    /// The folder the history is kept in.
    fn root<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<String> {
        Ok(RuntimeEnv::from_eval(eval)?
            .history
            .root()
            .to_string_lossy()
            .into_owned())
    }

    /// The records of past runs, newest first: at most `limit` of them, only
    /// those of task `kind`, those that ended `outcome` (`ok`, `failed`,
    /// `error`, `cancelled`, `running`, `abandoned`), those run on git
    /// `branch` and those run in `workspace` (its root, as
    /// `std.env.bazel_root_dir()` gives it), when given, and with `builds`,
    /// only those that started Bazel. Each is a dict, as `task.json` has it,
    /// its command line redacted, with `dir` (the entry's folder), `outcome`,
    /// and each build's `build_events` (its kept stream, for
    /// `bazel.build_events.read`) where its folder has one. The current
    /// run's own record is among them, `running`, unless its task keeps no
    /// history (`task(keep_history = False)`). Once `census()` has read the
    /// history, the records come from that read, each checked again.
    fn list<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = 50)] limit: u32,
        #[starlark(require = named, default = NoneOr::None)] kind: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] outcome: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] branch: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] workspace: NoneOr<String>,
        #[starlark(require = named, default = false)] builds: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        let workspace = workspace.into_option();
        let history = RuntimeEnv::from_eval(eval)?.history.clone();
        let (kind, wanted, branch) = (
            kind.into_option(),
            outcome.into_option(),
            branch.into_option(),
        );
        if let Some(o) = &wanted
            && !OUTCOMES.contains(&o.as_str())
        {
            anyhow::bail!(
                "unknown outcome {o:?}; the outcomes are {}",
                OUTCOMES.join(", ")
            );
        }
        // Whether each record kept is abandoned, read once, so its filter,
        // `status` and `outcome` agree.
        let seen = std::cell::RefCell::new(std::collections::HashMap::new());
        let records = history.records(limit as usize, |dir, r| {
            let gone = abandoned(dir, r);
            let keep = kind.as_ref().is_none_or(|k| &r.task.kind == k)
                && wanted.as_ref().is_none_or(|o| outcome_of(r, gone) == o)
                && branch
                    .as_ref()
                    .is_none_or(|b| r.git.as_ref().and_then(|g| g.branch.as_ref()) == Some(b))
                && workspace
                    .as_ref()
                    .is_none_or(|w| r.workspace.as_ref() == Some(w))
                && (!builds || !r.bazel.is_empty());
            seen.borrow_mut().insert(dir.to_path_buf(), gone);
            keep
        });
        let seen = seen.into_inner();
        Ok(records
            .iter()
            .map(|(dir, record)| {
                let gone = seen.get(dir).copied().unwrap_or(false);
                eval.heap().alloc(shown(dir, record, gone))
            })
            .collect())
    }

    /// How many runs the history keeps, as a dict: `runs`, those with a
    /// record that reads, which `list` lists; `without_record`, folders with
    /// no `task.json` (yet: a run starting); `newer`, records a newer CLI
    /// wrote, of a schema this one skips; and `unreadable`, those whose
    /// record is there but does not read. It reads every record once, and `list` and
    /// later calls answer from that read, a record rewritten since (a run
    /// that ended) read again.
    fn census<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let scan = RuntimeEnv::from_eval(eval)?.history.scan();
        Ok(eval.heap().alloc(serde_json::json!({
            "runs": scan.records.len(),
            "without_record": scan.without_record,
            "newer": scan.newer,
            "unreadable": scan.unreadable,
        })))
    }

    /// The output a run (`record`, as `list` gives it) kept, as it was shown
    /// (`output.ansi`, colour kept), read back as a live build's
    /// `build.output_feed()` gives lines; None when it kept none. Only the
    /// file's last 16 MiB is read, from its first whole line. A dict:
    /// `lines`, its last `limit` lines, oldest first, each a dict of `text`
    /// (escape sequences stripped), `links` (`[start, end, url]` by
    /// character of `text`) and `colour` (the same text, its colour kept);
    /// `dropped`, how many lines of what was read came before those; and
    /// `dropped_bytes`, how much of the file before what was read.
    fn read_output<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] record: Value<'v>,
        #[starlark(require = named, default = 5000)] limit: u32,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneOr<Value<'v>>> {
        let dir = starlark::values::dict::DictRef::from_value(record)
            .and_then(|d| d.get_str("dir"))
            .and_then(|v| v.unpack_str().map(str::to_owned))
            .ok_or_else(|| {
                anyhow::anyhow!("read_output: expected a record from list() or get()")
            })?;
        let path = std::path::Path::new(&dir).join(crate::out::OUTPUT_ANSI);
        let Ok(file) = std::fs::File::open(&path) else {
            return Ok(NoneOr::None);
        };
        let fail = |e: std::io::Error| anyhow::anyhow!("cannot read {}: {e}", path.display());
        let (bytes, dropped_bytes) = output_tail(file, OUTPUT_READ_MAX).map_err(fail)?;
        let text = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
        let all: Vec<&[u8]> = if text.is_empty() {
            vec![]
        } else {
            text.split(|&b| b == b'\n').collect()
        };
        let skip = all.len().saturating_sub(limit as usize);
        let lines: Vec<serde_json::Value> = all[skip..]
            .iter()
            .map(|raw| {
                let (text, links, colour) = bazel::console::feed::line_of(raw);
                serde_json::json!({"text": text, "links": links, "colour": colour})
            })
            .collect();
        Ok(NoneOr::Other(eval.heap().alloc(serde_json::json!({
            "lines": lines,
            "dropped": skip,
            "dropped_bytes": dropped_bytes,
        }))))
    }

    /// The record of the run with task ID `id` (or whose entry's folder is
    /// named `id`), as `list` gives it; None when it is not kept.
    fn get<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] id: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneOr<Value<'v>>> {
        let history = RuntimeEnv::from_eval(eval)?.history.clone();
        Ok(NoneOr::from_option(history.record(id).map(
            |(dir, record)| {
                let gone = abandoned(&dir, &record);
                eval.heap().alloc(shown(&dir, &record, gone))
            },
        )))
    }

    /// The run with task ID `id` (or whose entry's folder is named `id`)
    /// as a command to run it again: this CLI's own binary with the run's
    /// arguments as typed, not redacted, which the command never hands back
    /// (`std.process.Command`, ready for `current_dir`, streams and
    /// `spawn`). Before a `--` (the CLI's flags, not Bazel's arguments), the
    /// flags `drop` names are left out (`--x` takes `--x`, `--x=…`, `--x:…`;
    /// an entry with `=` only that flag exactly), and so is the argument after
    /// each one `drop_with_value` names, written without `=`; `add` goes in
    /// their place, before the `--`. None when the run is not kept, or its
    /// record's command line was redacted on disk (once its files were
    /// pruned), as running that would pass the redactions on.
    fn command<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] id: &str,
        #[starlark(require = named, default = UnpackList::default())] drop: UnpackList<String>,
        #[starlark(require = named, default = UnpackList::default())] drop_with_value: UnpackList<
            String,
        >,
        #[starlark(require = named, default = UnpackList::default())] add: UnpackList<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneOr<crate::engine::std::process::Command>> {
        let history = RuntimeEnv::from_eval(eval)?.history.clone();
        let Some((_, record)) = history.record(id).filter(|(_, r)| !r.argv_redacted) else {
            return Ok(NoneOr::None);
        };
        let args = again_args(
            &record.argv,
            &drop.items,
            &drop_with_value.items,
            &add.items,
        );
        let mut cmd = std::process::Command::new(std::env::current_exe()?);
        cmd.args(args);
        Ok(NoneOr::Other(
            crate::engine::std::process::Command::from_std(cmd, false),
        ))
    }

    /// Add a link to this run's record: a results page, say, which a history
    /// list then shows. A link already there is not added twice.
    fn link<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] label: &str,
        #[starlark(require = pos)] url: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        RuntimeEnv::from_eval(eval)?.history.link(label, url);
        Ok(NoneType)
    }
}

#[cfg(test)]
mod tests {
    /// A run's arguments to run it again: the program and the dropped
    /// flags left out before a `--`, the added ones in their place, Bazel's
    /// arguments after it as they were.
    #[test]
    fn a_runs_arguments_again() {
        let v = |s: &[&str]| s.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        let drop = v(&["--dashboard", "--bridge"]);
        let with_value = v(&["--dashboard:actions"]);
        let add = v(&["--bridge:enabled=true", "--dashboard:enabled=false"]);
        assert_eq!(
            super::again_args(
                &v(&[
                    "/bin/aspect",
                    "test",
                    "--dashboard:actions",
                    "all",
                    "--dashboard",
                    "//a",
                    "--",
                    "--dashboard"
                ]),
                &drop,
                &with_value,
                &add
            ),
            v(&[
                "test",
                "//a",
                "--bridge:enabled=true",
                "--dashboard:enabled=false",
                "--",
                "--dashboard"
            ])
        );
        assert_eq!(
            super::again_args(
                &v(&[
                    "/bin/aspect",
                    "build",
                    "--dashboard",
                    "--bridge:enabled=true",
                    "--dashboard:enabled=false",
                    "//a"
                ]),
                &v(&["--bridge", "--dashboard:enabled=false"]),
                &[],
                &[]
            ),
            v(&["build", "--dashboard", "//a"]),
            "an exact entry drops that flag alone"
        );
        assert_eq!(
            super::again_args(&[], &drop, &[], &[]),
            Vec::<String>::new()
        );
    }

    /// A long output is read from its first whole line in the limit, a line
    /// that starts right at the cut included, with how much came before.
    #[test]
    fn a_long_outputs_end_is_read_from_a_line() {
        let tail = |text: &str, max| {
            let mut f = tempfile::tempfile().unwrap();
            std::io::Write::write_all(&mut f, text.as_bytes()).unwrap();
            std::io::Seek::rewind(&mut f).unwrap();
            let (bytes, before) = super::output_tail(f, max).unwrap();
            (String::from_utf8(bytes).unwrap(), before)
        };
        assert_eq!(tail("one\ntwo\n", 8), ("one\ntwo\n".into(), 0));
        assert_eq!(tail("one\ntwo\nsix\n", 8), ("two\nsix\n".into(), 4));
        assert_eq!(tail("one\ntwo\nsix\n", 9), ("two\nsix\n".into(), 4));
        assert_eq!(tail("one\ntwo\nsix\n", 7), ("six\n".into(), 8));
        assert_eq!(tail("abcdefghijkl", 4), ("".into(), 12));
    }
}
