use std::io;
use std::process;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug, Default)]
struct SupervisorState {
    shutting_down: bool,
    pids: Vec<u32>,
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
        state.pids.push(pid);
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
    ) -> io::Result<(process::Child, ProcessGuard)> {
        self.register_spawn(|| command.spawn(), process::Child::id)
    }

    pub(crate) fn begin_shutdown(&self) {
        self.lock().shutting_down = true;
    }

    pub(crate) fn live_pids(&self) -> Vec<u32> {
        self.lock().pids.clone()
    }

    fn unregister(&self, pid: u32) {
        let mut state = self.lock();
        if let Some(index) = state.pids.iter().position(|candidate| *candidate == pid) {
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn guard_unregisters_process() {
        let supervisor = ProcessSupervisor::default();
        let (value, guard) = supervisor
            .register_spawn(|| Ok(42_u32), |pid| *pid)
            .unwrap();

        assert_eq!(value, 42);
        assert_eq!(supervisor.live_pids(), vec![42]);
        drop(guard);
        assert!(supervisor.live_pids().is_empty());
    }

    #[test]
    fn shutdown_rejects_new_spawns() {
        let supervisor = ProcessSupervisor::default();
        let called = AtomicBool::new(false);
        supervisor.begin_shutdown();

        let error = supervisor
            .register_spawn(
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
