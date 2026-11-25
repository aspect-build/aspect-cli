//! The operating system's half of stopping a process: one function per
//! intent, each a best-effort signal (one per process for a [`Target::Tree`],
//! whose members [`descendants`] lists), and the liveness probes the stop
//! sequences read. Unix sends signals; Windows sends a console break to
//! the child's own console group (children are spawned with
//! `CREATE_NEW_PROCESS_GROUP`, see [`creation_flags`]) and terminates
//! processes through a handle.

use std::process::Command;

/// What a signal is aimed at: one process, a whole process group (unix), or
/// a process and every process below it (macOS and Linux, which can list
/// them; elsewhere a tree is its root alone, and on Windows a group is too).
///
/// A tree is for a child that shares our process group, so no group signal
/// can single it out: `bazel` is often bazelisk, which runs the real client
/// as its child and does not pass a SIGINT on, and a `tools/bazel` wrapper
/// script that runs it rather than `exec` it is a shell, which defers a
/// SIGINT until its foreground child exits. Signalled alone, either holds
/// the SIGINT while the real client never sees one. Unlike a terminal's
/// Ctrl+C, a tree also reaches a descendant that left the group (`setsid`);
/// a process started after the members are listed, just before the signal,
/// is missed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Pid(u32),
    Group(i32),
    Tree(u32),
}

impl Target {
    pub fn pid(self) -> u32 {
        match self {
            Target::Pid(pid) | Target::Tree(pid) => pid,
            Target::Group(group) => group as u32,
        }
    }
}

/// Spawn-time configuration every child the runtime starts gets.
#[cfg(windows)]
pub fn creation_flags(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(windows))]
pub fn creation_flags(_cmd: &mut Command) {}

#[cfg(unix)]
mod imp {
    use super::Target;
    use nix::errno::Errno;
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;

    fn send(target: Target, signal: Option<Signal>) -> bool {
        let pid = match target {
            Target::Pid(pid) => Pid::from_raw(pid as i32),
            Target::Group(group) => Pid::from_raw(-group),
            Target::Tree(root) => {
                // Probing a tree is probing its root.
                if signal.is_some() {
                    for pid in super::descendants(root) {
                        let _ = signal::kill(Pid::from_raw(pid as i32), signal);
                    }
                }
                Pid::from_raw(root as i32)
            }
        };
        signal::kill(pid, signal).is_ok()
    }

    /// SIGINT: what Ctrl+C at a terminal delivers.
    pub fn interrupt(target: Target) -> bool {
        send(target, Some(Signal::SIGINT))
    }

    /// SIGTERM: please exit.
    pub fn terminate(target: Target) -> bool {
        send(target, Some(Signal::SIGTERM))
    }

    /// SIGKILL.
    pub fn kill(target: Target) -> bool {
        send(target, Some(Signal::SIGKILL))
    }

    /// Whether a process (or any member of a group) exists. A zombie counts,
    /// so for the runtime's own children prefer [`has_exited`].
    pub fn is_running(target: Target) -> bool {
        send(target, None)
    }

    /// Whether `pid`, a child of this process, has exited, whether or not it
    /// has been reaped. `waitid` with `WNOWAIT` peeks without reaping, so a
    /// later `Child::wait` still works; a pid that is not our child (already
    /// reaped) counts as exited.
    pub fn has_exited(pid: u32) -> bool {
        use nix::libc;
        // Zeroed, not uninitialized: with WNOHANG and nothing to report some
        // platforms leave the struct untouched, and a zero `si_signo` is how
        // "still running" reads below.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if rc != 0 {
            return Errno::last() == Errno::ECHILD;
        }
        info.si_signo == libc::SIGCHLD
    }
}

#[cfg(windows)]
mod imp {
    use super::Target;
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        TerminateProcess,
    };

    /// A console break to the child's own console group. Bazel's client
    /// treats it exactly like Ctrl+C; most console programs exit on it.
    fn console_break(target: Target) -> bool {
        unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, target.pid()) != 0 }
    }

    pub fn interrupt(target: Target) -> bool {
        console_break(target)
    }

    /// Windows has no graceful terminate; the console break is the closest.
    pub fn terminate(target: Target) -> bool {
        console_break(target)
    }

    pub fn kill(target: Target) -> bool {
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, target.pid());
            if handle.is_null() {
                return false;
            }
            let ok = TerminateProcess(handle, 1) != 0;
            CloseHandle(handle);
            ok
        }
    }

    pub fn is_running(target: Target) -> bool {
        !has_exited(target.pid())
    }

    pub fn has_exited(pid: u32) -> bool {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return true;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(handle, &mut code) != 0;
            CloseHandle(handle);
            !ok || code != STILL_ACTIVE as u32
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::Target;
    pub fn interrupt(_target: Target) -> bool {
        false
    }
    pub fn terminate(_target: Target) -> bool {
        false
    }
    pub fn kill(_target: Target) -> bool {
        false
    }
    pub fn is_running(_target: Target) -> bool {
        false
    }
    pub fn has_exited(_pid: u32) -> bool {
        true
    }
}

pub use imp::{has_exited, interrupt, is_running, kill, terminate};

/// Every process below `root`, children before their own children; empty
/// where the platform gives no way to list them.
pub fn descendants(root: u32) -> Vec<u32> {
    let children = children_lister();
    let mut found = vec![];
    let mut next = vec![root];
    while let Some(parent) = next.pop() {
        for child in children(parent) {
            // A cycle cannot happen in a process tree, but a pid reused
            // mid-walk could make one.
            if child != root && !found.contains(&child) {
                found.push(child);
                next.push(child);
            }
        }
    }
    found
}

/// A way to list a process's children: one syscall each on macOS.
#[cfg(target_os = "macos")]
fn children_lister() -> impl Fn(u32) -> Vec<u32> {
    |parent| {
        use nix::libc;
        let mut buf = vec![0 as libc::pid_t; 64];
        loop {
            let bytes = (buf.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
            // SAFETY: the buffer holds `bytes` bytes; the call writes at most
            // that, and returns how many pids it wrote.
            let n = unsafe {
                libc::proc_listchildpids(parent as libc::pid_t, buf.as_mut_ptr().cast(), bytes)
            };
            if n <= 0 {
                return vec![];
            }
            // A full buffer may have been cut short: ask again with room.
            if n as usize >= buf.len() {
                buf.resize(buf.len() * 2, 0);
                continue;
            }
            return buf[..n as usize]
                .iter()
                .filter(|&&p| p > 0)
                .map(|&p| p as u32)
                .collect();
        }
    }
}

/// A way to list a process's children: on Linux one scan of `/proc` for the
/// whole walk, each `stat`'s parent read once.
#[cfg(target_os = "linux")]
fn children_lister() -> impl Fn(u32) -> Vec<u32> {
    let mut by_parent: std::collections::HashMap<u32, Vec<u32>> = Default::default();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for pid in entries.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok()) {
            // `/proc/<pid>/stat`: the parent is the second field after the
            // command, which is in parentheses and may hold spaces.
            let parent = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|stat| {
                    let rest = &stat[stat.rfind(')')? + 1..];
                    rest.split_whitespace().nth(1)?.parse::<u32>().ok()
                });
            if let Some(parent) = parent {
                by_parent.entry(parent).or_default().push(pid);
            }
        }
    }
    move |parent| by_parent.get(&parent).cloned().unwrap_or_default()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn children_lister() -> impl Fn(u32) -> Vec<u32> {
    |_| vec![]
}

/// Best-effort one-line description of a running process for diagnostics:
/// its command line, plus its working directory where the platform exposes
/// one. `None` when the process is gone or cannot be inspected.
///
/// The description goes to CI logs, so the arguments pass through the same
/// redaction as an echoed bazel command (`--remote_header`, `--action_env`,
/// URL credentials).
#[cfg(target_os = "linux")]
pub fn describe_process(pid: u32) -> Option<String> {
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
pub fn describe_process(pid: u32) -> Option<String> {
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
    crate::bes::redact::redact_command_args(args).join(" ")
}

#[cfg(not(unix))]
pub fn describe_process(_pid: u32) -> Option<String> {
    None
}

/// Start `cmd` with SIGINT at its default disposition, whatever the test
/// process inherited: under `cargo test &` that is "ignored", and a child
/// that keeps it never acts on the SIGINT a test sends it.
#[cfg(all(test, unix))]
pub(crate) fn default_sigint(cmd: &mut Command) -> &mut Command {
    use std::os::unix::process::CommandExt;
    // SAFETY: the hook only calls `signal(2)`, which is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            use nix::sys::signal::{SigHandler, Signal, signal};
            signal(Signal::SIGINT, SigHandler::SigDfl)
                .map(drop)
                .map_err(std::io::Error::from)
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};

    fn sleeper() -> std::process::Child {
        default_sigint(&mut Command::new("sleep"))
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
    fn interrupt_reaches_the_process_and_reports_success() {
        let mut child = sleeper();
        assert!(interrupt(Target::Pid(child.id())));
        let status = child.wait().unwrap();
        assert_eq!(
            status.signal(),
            Some(nix::sys::signal::Signal::SIGINT as i32)
        );
    }

    #[test]
    fn terminate_reaches_the_process() {
        let mut child = sleeper();
        assert!(terminate(Target::Pid(child.id())));
        let status = child.wait().unwrap();
        assert_eq!(
            status.signal(),
            Some(nix::sys::signal::Signal::SIGTERM as i32)
        );
    }

    #[test]
    fn a_signal_to_a_missing_pid_reports_failure() {
        let mut child = Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        assert!(!interrupt(Target::Pid(child.id())));
    }

    #[test]
    fn is_running_tracks_the_process() {
        let mut child = sleeper();
        assert!(is_running(Target::Pid(child.id())));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!is_running(Target::Pid(child.id())));
    }

    #[test]
    fn has_exited_peeks_without_reaping() {
        let mut child = sleeper();
        assert!(!has_exited(child.id()));
        child.kill().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !has_exited(child.id()) {
            assert!(std::time::Instant::now() < deadline, "child never exited");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Still reapable: WNOWAIT left the status in place.
        assert!(child.wait().unwrap().signal().is_some());
        assert!(has_exited(child.id()));
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tree_tests {
    use super::*;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    /// A grandchild (root shell → shell → sleep) is listed under the root,
    /// and a tree signal reaches it.
    #[test]
    fn a_tree_signal_reaches_a_grandchild() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("grandchild.pid");
        let mut root = Command::new("sh")
            .arg("-c")
            .arg("sh -c 'sleep 30 & echo $! > \"$PID_FILE\"; wait'; true")
            .env("PID_FILE", &pid_file)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let grandchild = loop {
            if let Some(pid) = std::fs::read_to_string(&pid_file)
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .filter(|pid| descendants(root.id()).contains(pid))
            {
                break pid;
            }
            assert!(Instant::now() < deadline, "grandchild never listed");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(kill(Target::Tree(root.id())));
        root.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while is_running(Target::Pid(grandchild)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!is_running(Target::Pid(grandchild)), "grandchild survived");
    }
}
