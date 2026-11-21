//! `ctx.std.history`: the task history's records of past runs (each entry's
//! `task.json`; see `engine::task_record`), newest first, and the current
//! run's links.
//!
//! A record is handed out as a dict, its command line redacted, with two
//! keys of the reader's own: `dir`, the entry's folder, and, for a run whose
//! record says `running`, `status` set to `abandoned` once its process is no
//! longer alive, as after a crash. Records of a newer schema are skipped.

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

/// A record as handed out: redacted, with its folder, and a `running` one
/// whose process is gone marked `abandoned`.
fn shown(dir: &std::path::Path, record: &TaskRecord) -> serde_json::Value {
    let mut value = serde_json::to_value(record.redacted()).unwrap_or_default();
    value["dir"] = dir.to_string_lossy().into_owned().into();
    if record.status == "running" && record.pid != std::process::id() && !alive(record.pid) {
        value["status"] = "abandoned".into();
    }
    value
}

/// Whether a process `pid` is alive (one this user may not signal counts).
fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        // Signal 0 checks for the process without sending anything.
        match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
            Ok(()) | Err(nix::errno::Errno::EPERM) => true,
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

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

    /// The records of past runs, newest first: at most `limit` of them,
    /// only those of task `kind` when given. Each is a dict, as
    /// `task.json` has it, its command line redacted, with `dir`, the
    /// entry's folder. This run's own record is among them, `running`.
    fn list<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = 50)] limit: u32,
        #[starlark(require = named, default = NoneOr::None)] kind: NoneOr<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        let history = RuntimeEnv::from_eval(eval)?.history.clone();
        let kind = kind.into_option();
        let records = history.records(limit as usize, |r| {
            kind.as_ref().is_none_or(|k| &r.task.kind == k)
        });
        Ok(records
            .iter()
            .map(|(dir, record)| eval.heap().alloc(shown(dir, record)))
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
            |(dir, record)| eval.heap().alloc(shown(&dir, &record)),
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
