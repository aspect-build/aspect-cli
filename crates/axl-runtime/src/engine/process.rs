use std::io;
use std::process;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug, Default)]
struct SupervisorState {
    shutting_down: bool,
    /// (pid, owns its own process group) — a group leader is signaled with
    /// `killpg`, catching grandchildren a wrapper leaves behind.
    pids: Vec<(u32, bool)>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ProcessSupervisor {
    state: Arc<Mutex<SupervisorState>>,
}

impl ProcessSupervisor {
    fn lock(&self) -> MutexGuard<'_, SupervisorState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn register_spawn<T>(
        &self,
        own_group: bool,
        spawn: impl FnOnce() -> io::Result<T>,
        pid: impl FnOnce(&T) -> u32,
    ) -> io::Result<(T, ProcessGuard)> {
        let mut state = self.lock();
        if state.shutting_down {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "process shutdown is already in progress",
            ));
        }

        let child = spawn()?;
        let pid = pid(&child);
        state.pids.push((pid, own_group));
        Ok((
            child,
            ProcessGuard {
                supervisor: self.clone(),
                pid,
            },
        ))
    }

    pub(crate) fn spawn(
        &self,
        command: &mut process::Command,
        own_group: bool,
    ) -> io::Result<(process::Child, ProcessGuard)> {
        self.register_spawn(own_group, || command.spawn(), process::Child::id)
    }

    pub(crate) fn begin_shutdown(&self) {
        self.lock().shutting_down = true;
    }

    pub(crate) fn live_pids(&self) -> Vec<u32> {
        self.lock().pids.iter().map(|(pid, _)| *pid).collect()
    }

    pub(crate) fn entries(&self) -> Vec<(u32, bool)> {
        self.lock().pids.clone()
    }

    fn unregister(&self, pid: u32) {
        let mut state = self.lock();
        if let Some(index) = state
            .pids
            .iter()
            .position(|(candidate, _)| *candidate == pid)
        {
            state.pids.swap_remove(index);
        }
    }
}

#[derive(Debug)]
#[must_use = "the process is tracked only while the guard is alive"]
pub struct ProcessGuard {
    supervisor: ProcessSupervisor,
    pid: u32,
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        self.supervisor.unregister(self.pid);
    }
}

#[cfg(unix)]
pub(crate) fn is_pid_running(pid: u32) -> bool {
    use nix::sys::signal;
    use nix::unistd::Pid;

    signal::kill(Pid::from_raw(pid as i32), None).is_ok()
}

#[cfg(not(unix))]
pub(crate) fn is_pid_running(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
pub(crate) fn sigkill(pid: u32) -> bool {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;

    signal::kill(Pid::from_raw(pid as i32), Signal::SIGKILL).is_ok()
}

#[cfg(not(unix))]
pub(crate) fn sigkill(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
pub(crate) fn sigint(pid: u32) -> bool {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;

    signal::kill(Pid::from_raw(pid as i32), Signal::SIGINT).is_ok()
}

#[cfg(not(unix))]
pub(crate) fn sigint(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
pub(crate) fn sigterm(pid: u32) -> bool {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;

    signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM).is_ok()
}

#[cfg(not(unix))]
pub(crate) fn sigterm(_pid: u32) -> bool {
    false
}

/// SIGTERM to the whole process group led by `pid`. An already-gone group
/// (`ESRCH`) is success — the goal is reached; any other error propagates.
#[cfg(unix)]
pub(crate) fn sigterm_group(pid: u32) -> std::io::Result<()> {
    killpg(pid, nix::sys::signal::Signal::SIGTERM)
}

#[cfg(not(unix))]
pub(crate) fn sigterm_group(_pid: u32) -> std::io::Result<()> {
    Ok(())
}

/// SIGKILL to the whole process group led by `pid`. `ESRCH` (already gone) is
/// success; any other error propagates.
#[cfg(unix)]
pub(crate) fn sigkill_group(pid: u32) -> std::io::Result<()> {
    killpg(pid, nix::sys::signal::Signal::SIGKILL)
}

#[cfg(not(unix))]
pub(crate) fn sigkill_group(_pid: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn killpg(pid: u32, signal: nix::sys::signal::Signal) -> std::io::Result<()> {
    match nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid as i32), signal) {
        Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn guard_unregisters_process() {
        let supervisor = ProcessSupervisor::default();
        let (value, guard) = supervisor
            .register_spawn(false, || Ok(42_u32), |pid| *pid)
            .unwrap();

        assert_eq!(value, 42);
        assert_eq!(supervisor.live_pids(), vec![42]);
        drop(guard);
        assert!(supervisor.live_pids().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn group_signal_reaches_grandchildren() {
        use std::time::{Duration, Instant};

        let mut cmd = process::Command::new("sh");
        cmd.args(["-c", "sleep 30 & wait"]);
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn().unwrap();
        let pid = child.id();

        sigterm_group(pid).unwrap();
        child.wait().unwrap();

        // A plain SIGTERM to the shell would leave the background `sleep`
        // holding the group for its full 30s; the group signal empties it.
        let deadline = Instant::now() + Duration::from_secs(5);
        while nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid as i32), None).is_ok() {
            assert!(Instant::now() < deadline, "process group never emptied");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn shutdown_rejects_new_spawns() {
        let supervisor = ProcessSupervisor::default();
        let called = AtomicBool::new(false);
        supervisor.begin_shutdown();

        let error = supervisor
            .register_spawn(
                false,
                || {
                    called.store(true, Ordering::Relaxed);
                    Ok(42_u32)
                },
                |pid| *pid,
            )
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(!called.load(Ordering::Relaxed));
    }
}
