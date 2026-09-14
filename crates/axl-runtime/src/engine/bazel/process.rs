/// Probes whether a process with the given PID exists using signal 0.
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

/// Sends SIGKILL to the given PID. Silently ignores failures.
#[cfg(unix)]
pub(crate) fn sigkill(pid: u32) {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;

    tracing::warn!("Sending SIGKILL to PID {}", pid);
    let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
}

#[cfg(not(unix))]
pub(crate) fn sigkill(_pid: u32) {
    tracing::warn!("sigkill is not supported on this platform");
}

/// Sends SIGINT to the given PID. Returns true if the signal was sent successfully.
#[cfg(unix)]
pub(crate) fn sigint(pid: u32) -> bool {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;

    signal::kill(Pid::from_raw(pid as i32), Signal::SIGINT).is_ok()
}

#[cfg(not(unix))]
pub(crate) fn sigint(_pid: u32) -> bool {
    tracing::warn!("sigint is not supported on this platform");
    false
}

/// Sends SIGTERM to the given PID. Returns true if the signal was sent successfully.
#[cfg(unix)]
pub(crate) fn sigterm(pid: u32) -> bool {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;

    signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM).is_ok()
}

#[cfg(not(unix))]
pub(crate) fn sigterm(_pid: u32) -> bool {
    tracing::warn!("sigterm is not supported on this platform");
    false
}

/// Best-effort one-line description of a running process for diagnostics:
/// its command line, plus its working directory where the platform exposes
/// one. `None` when the process is gone or cannot be inspected.
#[cfg(target_os = "linux")]
pub(crate) fn describe_process(pid: u32) -> Option<String> {
    let proc_dir = std::path::PathBuf::from(format!("/proc/{pid}"));
    let raw = std::fs::read(proc_dir.join("cmdline")).ok()?;
    let cmdline: Vec<String> = raw
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();
    if cmdline.is_empty() {
        return None;
    }
    let mut desc = cmdline.join(" ");
    if let Ok(cwd) = std::fs::read_link(proc_dir.join("cwd")) {
        desc.push_str(&format!(" (cwd {})", cwd.display()));
    }
    Some(desc)
}

/// See the Linux variant. Without procfs, `ps` gives the command line and
/// nothing reports the working directory.
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn describe_process(pid: u32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let desc = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() && !desc.is_empty() {
        Some(desc)
    } else {
        None
    }
}

#[cfg(not(unix))]
pub(crate) fn describe_process(_pid: u32) -> Option<String> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};

    fn sleeper() -> std::process::Child {
        Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep")
    }

    #[test]
    fn describe_process_names_a_live_process() {
        let mut child = sleeper();
        let desc = describe_process(child.id());
        child.kill().unwrap();
        child.wait().unwrap();
        let desc = desc.expect("live process is describable");
        assert!(desc.contains("sleep"), "{desc}");
        assert!(desc.contains("30"), "{desc}");
    }

    #[test]
    fn describe_process_is_none_for_an_exited_process() {
        let mut child = Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        assert_eq!(describe_process(child.id()), None);
    }

    #[test]
    fn sigterm_terminates_and_reports_success() {
        let mut child = sleeper();
        assert!(sigterm(child.id()));
        let status = child.wait().unwrap();
        assert_eq!(status.signal(), Some(libc_sigterm()));
    }

    #[test]
    fn sigterm_to_a_missing_pid_reports_failure() {
        let mut child = Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        assert!(!sigterm(child.id()));
    }

    #[test]
    fn is_pid_running_tracks_the_process() {
        let mut child = sleeper();
        assert!(is_pid_running(child.id()));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!is_pid_running(child.id()));
    }

    fn libc_sigterm() -> i32 {
        nix::sys::signal::Signal::SIGTERM as i32
    }
}
