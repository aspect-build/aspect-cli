//! The context `BazelWorkspaceStatusAction` asks the action context registry for.
//!
//! `BazelWorkspaceStatusModule` registers its `WorkspaceStatusAction.Context`
//! from `registerActionContexts(registry, env, request)`, over a
//! `CommandEnvironment`; the class is private and takes that environment, so the
//! three methods are written again here over what the command would have read
//! from it: the option class from the build's parser, the client environment,
//! and the `--workspace_status_command` as a shell command in the workspace.
//! The action itself, its artifacts and its `BuildInfoEvent` are Bazel's.

use super::*;
use blazon_rt::Result as RtResult;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) const WORKSPACE_STATUS_CONTEXT: &str =
    "com/google/devtools/build/lib/analysis/WorkspaceStatusAction$Context";
const WORKSPACE_STATUS_OPTIONS: &str =
    "com/google/devtools/build/lib/analysis/WorkspaceStatusAction$Options";

/// Whether the action ran in a build: set when it asks for its options, which
/// is the first thing `BazelWorkspaceStatusAction.execute` does.
#[derive(Clone, Default)]
pub struct Ran(Arc<AtomicBool>, Arc<AtomicBool>);

impl Ran {
    pub fn get(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn set(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether the action failed, so Bazel posted no status.
    pub fn did_fail(&self) -> bool {
        self.1.load(Ordering::Acquire)
    }

    pub fn failed(&self) {
        self.1.store(true, Ordering::Release);
    }
}

pub(super) struct Context {
    pub parser: Global<bazel::OptionsParser<'static>>,
    pub workspace: Global<bazel::Path<'static>>,
    pub ran: Ran,
}

impl bazel::WorkspaceStatusContextImpl for Context {
    fn options<'a>(&self, env: &mut JNIEnv<'a>) -> RtResult<bazel::WorkspaceStatusOptions<'a>> {
        self.ran.set();
        let parser = self.parser.get(env)?;
        let class =
            bazel::Class::<bazel::WorkspaceStatusOptions>::find(env, WORKSPACE_STATUS_OPTIONS)?;
        parser.get_options(&class)
    }

    fn client_env<'a>(
        &self,
        env: &mut JNIEnv<'a>,
    ) -> RtResult<bazel::ImmutableMap<'a, bazel::JavaString<'static>, bazel::JavaString<'static>>>
    {
        client_env(env, false).map_err(|e| blazon_rt::Error::msg(format!("{e:#}")))
    }

    fn command<'a>(&self, env: &mut JNIEnv<'a>) -> RtResult<Option<bazel::Command<'a>>> {
        let parser = self.parser.get(env)?;
        let class =
            bazel::Class::<bazel::WorkspaceStatusOptions>::find(env, WORKSPACE_STATUS_OPTIONS)?;
        let options: bazel::WorkspaceStatusOptions = parser.get_options(&class)?;
        let command = options.workspace_status_command()?;
        if command.is_empty()? {
            return Ok(None);
        }
        let environment =
            client_env(env, false).map_err(|e| blazon_rt::Error::msg(format!("{e:#}")))?;
        let args = bazel::List::of_strs(env, &[command.path_string()?])?;
        Ok(Some(
            bazel::CommandBuilder::new(env, &environment)?
                .add_args(&args)?
                .set_env(&environment)?
                .set_working_dir(&self.workspace.get(env)?)?
                .use_shell(true)?
                .build()?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::settings;

    #[test]
    fn the_schema_has_the_status_settings() {
        for name in ["workspace_status_command", "embed_label", "stamp"] {
            let (def, renamed) = settings::lookup(name).unwrap();
            assert_eq!(def.name, name);
            assert!(renamed.is_none());
        }
    }

    #[test]
    fn a_status_command_is_a_path_and_a_label_a_string() {
        let (command, _) = settings::lookup("workspace_status_command").unwrap();
        let value = settings::Value::Str("tools/status.sh".into());
        assert_eq!(
            settings::arguments(command, &value).unwrap(),
            ["--workspace_status_command=tools/status.sh"]
        );
        let (label, _) = settings::lookup("embed_label").unwrap();
        assert!(settings::arguments(label, &settings::Value::Bool(true)).is_err());
    }

    #[test]
    fn ran_and_failed_are_separate_and_shared_between_clones() {
        let ran = super::Ran::default();
        let seen = ran.clone();
        assert!(!seen.get() && !seen.did_fail());
        ran.set();
        assert!(seen.get() && !seen.did_fail());
        ran.failed();
        assert!(seen.did_fail());
    }
}
