//! `ctx.std.history`: the task history's records of past runs (each entry's
//! `task.json`; see `engine::task_record`), newest first, and the current
//! run's links.
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
    /// `error`, `cancelled`, `running`, `abandoned`), and those run on git
    /// `branch`, when given. Each is a dict, as `task.json` has it, its
    /// command line redacted, with `dir` (the entry's folder), `outcome`, and
    /// each build's `build_events` (its kept stream, for
    /// `bazel.build_events.read`) where its folder has one. This run's own
    /// record is among them, `running`.
    fn list<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = 50)] limit: u32,
        #[starlark(require = named, default = NoneOr::None)] kind: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] outcome: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] branch: NoneOr<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
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
                    .is_none_or(|b| r.git.as_ref().and_then(|g| g.branch.as_ref()) == Some(b));
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
