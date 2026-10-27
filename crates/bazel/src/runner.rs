//! How a bazel client gets run. That is the host's to decide: which binary,
//! whether a spawned client is tracked so a cancel can stop it, and when the
//! work should give up. Everything that drives bazel is written against
//! [`Runner`] and so never reads the environment or knows about cancellation.

use std::io;
use std::process::{Command, ExitStatus, Output};

/// A spawned bazel client, waited on however the host's cancellation wants.
///
/// A wait that was cancelled returns the host's own error, unchanged: callers
/// pass a wait's error along and never re-wrap it, which is what lets the host
/// recognise its cancellation at the far end.
pub trait Child {
    fn wait(&self) -> io::Result<ExitStatus>;
    fn wait_with_output(self: Box<Self>) -> io::Result<Output>;
}

pub trait Runner {
    /// Which bazel to run: a name the OS resolves through `PATH`, or a path.
    /// A host overrides this to honour its own configuration (the runtime
    /// reads `BAZEL_REAL`, the bazelisk convention).
    fn binary(&self) -> String {
        "bazel".to_string()
    }

    /// The command every bazel invocation starts from: [`binary`](Self::binary)
    /// plus `ASPECT_CLI_RUNNING=1`.
    ///
    /// When a `tools/bazel` wrapper forwards to `aspect` for build/test/run,
    /// aspect then spawns its own child bazel. If `PATH` still has
    /// `tools/bazel` first, that child re-enters the wrapper and bounces back
    /// into aspect, forever. Cooperating wrappers check for the variable on
    /// entry and go straight to the real bazel, so every spawn must carry it.
    /// Hosts do not override this, so no spawn can miss the guard.
    fn command(&self) -> Command {
        let mut cmd = Command::new(self.binary());
        cmd.env("ASPECT_CLI_RUNNING", "1");
        cmd
    }

    /// Start `cmd`, registered with whatever stops it on cancel.
    fn spawn(&self, cmd: &mut Command) -> io::Result<Box<dyn Child>>;

    /// Whether the work should stop. Polled by anything that loops.
    fn cancelled(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Plain;

    impl Runner for Plain {
        fn spawn(&self, _cmd: &mut Command) -> io::Result<Box<dyn Child>> {
            Err(io::Error::other("not spawned in this test"))
        }

        fn cancelled(&self) -> bool {
            false
        }
    }

    struct Custom;

    impl Runner for Custom {
        fn binary(&self) -> String {
            "/opt/bazel/real".to_string()
        }

        fn spawn(&self, _cmd: &mut Command) -> io::Result<Box<dyn Child>> {
            Err(io::Error::other("not spawned in this test"))
        }

        fn cancelled(&self) -> bool {
            false
        }
    }

    fn guard(cmd: &Command) -> Option<Option<String>> {
        cmd.get_envs()
            .find(|(k, _)| *k == "ASPECT_CLI_RUNNING")
            .map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    #[test]
    fn the_default_binary_is_bazel_and_carries_the_anti_inception_guard() {
        let cmd = Plain.command();
        assert_eq!(cmd.get_program(), "bazel");
        assert_eq!(guard(&cmd), Some(Some("1".to_string())));
    }

    #[test]
    fn a_host_that_picks_the_binary_still_gets_the_guard() {
        let cmd = Custom.command();
        assert_eq!(cmd.get_program(), "/opt/bazel/real");
        assert_eq!(guard(&cmd), Some(Some("1".to_string())));
    }
}
