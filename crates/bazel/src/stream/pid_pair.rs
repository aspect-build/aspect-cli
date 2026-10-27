//! One bazel command, as the process tree sees it.
//!
//! A command has two processes that matter and they end at different times:
//! the **client** that ran it, which exits when the command does, and the
//! **server** that did the work and writes the outputs, which outlives it by
//! design and can go on flushing after the client is gone. Only the client
//! can say "this is over"; only the server knows what is still being written.

use std::path::{Path, PathBuf};

use super::Gate;
use crate::process::{is_path_open_for_pid, is_pid_alive};

/// The two processes of one bazel command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PidPair {
    /// The process that ran the command: bazel's own client, the one we spawn.
    pub client: u32,
    /// The server, when known. Only a hint, read before the build, so it can
    /// be stale if the server restarts for the command. Read it with the
    /// build's startup flags, so it is the server this command will use.
    ///
    /// What a wrong answer costs depends on the stream. For a pipe it costs
    /// speed: an empty read already means "no writer attached", and the
    /// client's exit is what ends the stream. For a regular file it can cost
    /// events: nothing but the server's open files says the writer is still
    /// writing, so a stale pid makes the file look finished early.
    pub server: Option<u32>,
}

impl PidPair {
    /// The two answers a stream on `path` needs (see [`Gate`]):
    ///
    /// * `ended`: the client is gone (a zombie counts as gone).
    /// * `held`: the server has `path` open. Absent without a server pid. If
    ///   the server's fds cannot be read the answer is "held", the cautious
    ///   one, since "not held" ends streams early.
    pub fn gate(&self, path: &Path) -> Gate {
        let client = self.client;
        let gate = Gate::new(move || !is_pid_alive(client));
        match self.server {
            Some(server) => {
                let path = resolve(path);
                gate.with_held(move || is_path_open_for_pid(&path, server).unwrap_or(true))
            }
            None => gate,
        }
    }
}

/// `path` as an fd table lists it (`/tmp` is `/private/tmp` on macOS). A file
/// bazel has not created yet cannot be resolved itself, so its directory is.
fn resolve(path: &Path) -> PathBuf {
    if let Ok(resolved) = path.canonicalize() {
        return resolved;
    }
    match (
        path.parent().and_then(|d| d.canonicalize().ok()),
        path.file_name(),
    ) {
        (Some(dir), Some(name)) => dir.join(name),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::pipe::Channel;
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    fn sleeper() -> Child {
        Command::new("sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep")
    }

    fn reaped_pid() -> u32 {
        let mut child = Command::new("true").spawn().expect("spawn true");
        let pid = child.id();
        child.wait().expect("reap");
        pid
    }

    #[test]
    fn the_client_is_not_ended_while_it_runs() {
        let mut client = sleeper();
        let mut gate = PidPair {
            client: client.id(),
            server: None,
        }
        .gate(Path::new("/nonexistent"));
        assert!(!(gate.ended)());
        let _ = client.kill();
        let _ = client.wait();
    }

    #[test]
    fn the_client_is_ended_once_it_is_gone() {
        let mut gate = PidPair {
            client: reaped_pid(),
            server: None,
        }
        .gate(Path::new("/nonexistent"));
        assert!((gate.ended)());
    }

    #[test]
    fn an_exited_client_nobody_reaped_is_ended() {
        // The zombie state a client is in while its parent is still busy
        // reading its output.
        let mut client = sleeper();
        let mut gate = PidPair {
            client: client.id(),
            server: None,
        }
        .gate(Path::new("/nonexistent"));
        unsafe { libc::kill(client.id() as libc::pid_t, libc::SIGKILL) };
        std::thread::sleep(Duration::from_millis(300));
        assert!((gate.ended)(), "a zombie must read as ended");
        let _ = client.wait();
    }

    #[test]
    fn without_a_server_pid_there_is_no_held_hint() {
        let gate = PidPair {
            client: std::process::id(),
            server: None,
        }
        .gate(Path::new("/nonexistent"));
        assert!(gate.held.is_none());
    }

    #[test]
    fn held_follows_whether_the_server_has_the_path_open() {
        let channel = Channel::at(std::env::temp_dir().join(format!(
            "bazel-invocation-test-{}-{}.fifo",
            std::process::id(),
            uuid::Uuid::new_v4()
        )))
        .unwrap();
        let path = channel.path().to_path_buf();

        // This process plays the server.
        let mut gate = PidPair {
            client: std::process::id(),
            server: Some(std::process::id()),
        }
        .gate(&path);
        let held = gate.held.as_mut().expect("a server pid gives a hint");
        assert!(!held(), "nothing has the FIFO open yet");

        let reader = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
            .unwrap();
        assert!(held(), "the server holds it now");

        drop(reader);
        assert!(!held(), "and no longer");
    }

    #[test]
    fn held_is_asked_about_the_resolved_path() {
        let dir =
            std::env::temp_dir().join(format!("bazel-invocation-dir-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let link =
            std::env::temp_dir().join(format!("bazel-invocation-link-{}", uuid::Uuid::new_v4()));
        std::os::unix::fs::symlink(&dir, &link).unwrap();

        let channel = Channel::at(dir.join("pipe.fifo")).unwrap();
        let reader = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(channel.path())
            .unwrap();

        // Ask through the symlinked spelling of the same path.
        let mut gate = PidPair {
            client: std::process::id(),
            server: Some(std::process::id()),
        }
        .gate(&link.join("pipe.fifo"));
        assert!((gate.held.as_mut().unwrap())());

        drop(reader);
        drop(channel);
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_not_created_yet_is_resolved_through_its_directory() {
        let dir = std::env::temp_dir().join(format!("bazel-pid-pair-dir-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let link =
            std::env::temp_dir().join(format!("bazel-pid-pair-link-{}", uuid::Uuid::new_v4()));
        std::os::unix::fs::symlink(&dir, &link).unwrap();

        // Asked through the symlink, before the file exists.
        let mut gate = PidPair {
            client: std::process::id(),
            server: Some(std::process::id()),
        }
        .gate(&link.join("exec.log"));
        let held = gate.held.as_mut().unwrap();
        assert!(!held());

        let file = std::fs::File::create(dir.join("exec.log")).unwrap();
        assert!(
            held(),
            "the server has it open now, whichever way it was spelled"
        );
        drop(file);
        assert!(!held());

        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_path_that_cannot_be_resolved_at_all_is_kept_as_given() {
        assert_eq!(
            resolve(Path::new("/definitely/not/here/x.log")),
            PathBuf::from("/definitely/not/here/x.log")
        );
    }

    #[test]
    fn a_dead_server_holds_nothing() {
        let channel = Channel::at(std::env::temp_dir().join(format!(
            "bazel-invocation-dead-{}.fifo",
            uuid::Uuid::new_v4()
        )))
        .unwrap();
        let mut gate = PidPair {
            client: std::process::id(),
            server: Some(reaped_pid()),
        }
        .gate(channel.path());
        assert!(!(gate.held.as_mut().unwrap())());
    }

    #[test]
    fn the_gate_keeps_the_default_windows() {
        let gate = PidPair {
            client: 1,
            server: None,
        }
        .gate(Path::new("/nonexistent"));
        assert_eq!(gate.quiet, crate::stream::QUIET);
        assert_eq!(gate.quiet_empty, crate::stream::QUIET_EMPTY);
    }
}
