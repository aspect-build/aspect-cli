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

#[cfg(not(unix))]
pub(crate) fn sigterm(_pid: u32) -> bool {
    tracing::warn!("sigterm is not supported on this platform");
    false
}

/// Best-effort one-line description of a running process for diagnostics:
/// its command line, plus its working directory where the platform exposes
/// one. `None` when the process is gone or cannot be inspected.
///
/// The description goes to CI logs, so the arguments pass through the same
/// redaction as an echoed bazel command (`--remote_header`, `--action_env`,
/// URL credentials).
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
    let mut desc = redact_args(cmdline.iter().map(String::as_str));
    if let Ok(cwd) = std::fs::read_link(proc_dir.join("cwd")) {
        desc.push_str(&format!(" (cwd {})", cwd.display()));
    }
    Some(desc)
}

/// See the Linux variant. Without procfs there is no way to recover argv
/// boundaries for redaction (`ps` flattens them into one string), so only
/// the executable is reported, and no working directory.
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn describe_process(pid: u32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let exe = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || exe.is_empty() {
        return None;
    }
    Some(exe)
}

/// Join `args` into one line with credential-bearing values scrubbed.
#[cfg(target_os = "linux")]
fn redact_args<'a>(args: impl IntoIterator<Item = &'a str> + Clone) -> String {
    super::stream::redaction::redact_command_args(args).join(" ")
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

    /// A shell carrying `args` in its argv that prints `ready` once it is
    /// running, then blocks on stdin. Reading the line before inspecting
    /// the process rules out catching it mid-exec, when `/proc/<pid>/cmdline`
    /// is empty or still the parent's.
    fn ready_holder(args: &[&str]) -> std::process::Child {
        use std::io::{BufRead, BufReader};
        let mut child = Command::new("sh")
            .args(["-c", "echo ready; read _", "sh"])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sh");
        let mut line = String::new();
        BufReader::new(child.stdout.as_mut().unwrap())
            .read_line(&mut line)
            .expect("read ready line");
        assert_eq!(line.trim(), "ready");
        child
    }

    fn describe_then_kill(child: &mut std::process::Child) -> String {
        let desc = describe_process(child.id());
        child.kill().unwrap();
        child.wait().unwrap();
        desc.expect("live process is describable")
    }

    #[test]
    fn describe_process_names_a_live_process() {
        let mut child = ready_holder(&["--marker=visible"]);
        let desc = describe_then_kill(&mut child);
        assert!(desc.contains("sh"), "{desc}");
        if cfg!(target_os = "linux") {
            assert!(desc.contains("--marker=visible"), "{desc}");
            assert!(desc.contains("(cwd "), "{desc}");
        }
    }

    #[test]
    fn describe_process_is_none_for_an_exited_process() {
        let mut child = Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        assert_eq!(describe_process(child.id()), None);
    }

    #[test]
    fn describe_process_redacts_credentials() {
        let mut child = ready_holder(&["--remote_header=Authorization: Bearer hunter2"]);
        let desc = describe_then_kill(&mut child);
        assert!(!desc.contains("hunter2"), "{desc}");
        if cfg!(target_os = "linux") {
            assert!(desc.contains("--remote_header="), "{desc}");
        }
    }

    #[test]
    fn sigint_interrupts_and_reports_success() {
        let mut child = sleeper();
        assert!(sigint(child.id()));
        let status = child.wait().unwrap();
        assert_eq!(
            status.signal(),
            Some(nix::sys::signal::Signal::SIGINT as i32)
        );
    }

    #[test]
    fn sigint_to_a_missing_pid_reports_failure() {
        let mut child = Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        assert!(!sigint(child.id()));
    }

    #[test]
    fn is_pid_running_tracks_the_process() {
        let mut child = sleeper();
        assert!(is_pid_running(child.id()));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!is_pid_running(child.id()));
    }
}
