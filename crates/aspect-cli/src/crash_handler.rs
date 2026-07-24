//! Fatal-signal crash reporter.
//!
//! Installs handlers for the fatal signals (SIGSEGV, SIGBUS, SIGILL, SIGFPE,
//! SIGABRT) that print the signal name, fault address, and a best-effort
//! backtrace to stderr, then re-raise the signal with its default disposition
//! so the process still dies with the original signal and exit statuses are
//! unchanged.
//!
//! Motivation: a native crash on a CI runner otherwise surfaces only as a bare
//! `Received "segmentation fault" signal` line from the CI harness, with no
//! stack to debug from — kernel logs and core dumps on ephemeral runners are
//! usually gone before anyone can look. This lands a crash report in the task
//! log itself.
//!
//! Async-signal-safety: capturing and symbolizing a backtrace allocates, which
//! is not async-signal-safe (the crashing thread may hold the allocator lock).
//! This is a deliberate best-effort trade — the process is already dying. Two
//! guards contain the risk: a fixed marker naming the signal is written with
//! raw `write(2)` before the allocating section, so the signal is identified
//! even if backtrace capture wedges; and a re-entrancy flag turns any fault
//! inside the handler into an immediate default-action death. Handlers run on
//! the alternate signal stack (`SA_ONSTACK`) that Rust's std installs per
//! thread, so stack-overflow SIGSEGVs are reported rather than double-faulting.
//!
//! `ASPECT_NO_CRASH_HANDLER` (any non-empty value) skips installation.

/// Install the fatal-signal handlers. Call first thing in `main`, before any
/// runtime machinery, so the reporter covers everything after it. No-op on
/// non-unix platforms and under `ASPECT_NO_CRASH_HANDLER`.
pub fn install() {
    #[cfg(unix)]
    unix::install();
}

/// Test hook: crash the process immediately when `ASPECT_INTERNAL_TEST_CRASH`
/// is `segv` or `abort`, otherwise a no-op. Lets the end-to-end tests drive a
/// real crash through a spawned binary. Kept separate from [`install`] so a
/// test can exercise the `ASPECT_NO_CRASH_HANDLER` opt-out (handler skipped)
/// while still triggering the crash.
#[doc(hidden)]
pub fn trigger_test_crash() {
    #[cfg(unix)]
    unix::trigger_test_crash();
}

#[cfg(unix)]
mod unix {
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Environment variable that, when set to any non-empty value, skips
    /// handler installation.
    const OPT_OUT_ENV: &str = "ASPECT_NO_CRASH_HANDLER";

    /// The signals we install handlers for, paired with the label printed in
    /// the crash report.
    const FATAL_SIGNALS: &[(libc::c_int, &str)] = &[
        (libc::SIGSEGV, "SIGSEGV (segmentation fault)"),
        (libc::SIGBUS, "SIGBUS (bus error)"),
        (libc::SIGILL, "SIGILL (illegal instruction)"),
        (libc::SIGFPE, "SIGFPE (arithmetic exception)"),
        (libc::SIGABRT, "SIGABRT (abort)"),
    ];

    /// Set once a handler is running. A second entry — a fault inside the
    /// handler, or a concurrent crash on another thread — skips the report and
    /// goes straight to the default action.
    static HANDLING: AtomicBool = AtomicBool::new(false);

    /// Whether the opt-out environment variable is set to a non-empty value.
    fn opt_out() -> bool {
        std::env::var_os(OPT_OUT_ENV).is_some_and(|v| !v.is_empty())
    }

    pub(super) fn install() {
        if opt_out() {
            return;
        }
        let f: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) = handler;
        for &(sig, _) in FATAL_SIGNALS {
            // SAFETY: standard sigaction registration; `sa` is fully
            // initialized before use. `sa_mask` blocks the other fatal signals
            // for the duration of the handler so a sibling signal can't
            // re-enter it (the HANDLING guard is the backstop if one does).
            unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = f as usize;
                sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut sa.sa_mask);
                for &(other, _) in FATAL_SIGNALS {
                    libc::sigaddset(&mut sa.sa_mask, other);
                }
                libc::sigaction(sig, &sa, std::ptr::null_mut());
            }
        }
    }

    pub(super) fn trigger_test_crash() {
        match std::env::var("ASPECT_INTERNAL_TEST_CRASH").as_deref() {
            // SAFETY: intentional null write to raise SIGSEGV.
            Ok("segv") => unsafe { std::ptr::null_mut::<u8>().write_volatile(1) },
            Ok("abort") => std::process::abort(),
            _ => {}
        }
    }

    fn signal_name(sig: libc::c_int) -> &'static str {
        FATAL_SIGNALS
            .iter()
            .find(|(s, _)| *s == sig)
            .map(|(_, name)| *name)
            .unwrap_or("unknown fatal signal")
    }

    /// The report body written after the signal-name marker. Pure so it can be
    /// unit-tested; the handler supplies the live fault address and thread name.
    fn format_report(fault_addr: usize, thread_name: &str) -> String {
        format!(
            "fault address: {fault_addr:#x}\nthread: {thread_name}\nbacktrace:\n{}\n\
             aspect-cli crashed; please report this at \
             https://github.com/aspect-build/aspect-cli/issues including the output above.\n",
            std::backtrace::Backtrace::force_capture(),
        )
    }

    /// Write `bytes` to stderr via `write(2)`, ignoring errors. Async-signal-safe.
    fn write_stderr(bytes: &[u8]) {
        let mut off = 0;
        while off < bytes.len() {
            // SAFETY: in-bounds pointer/length into `bytes`, written to fd 2.
            let n = unsafe {
                libc::write(
                    libc::STDERR_FILENO,
                    bytes[off..].as_ptr().cast(),
                    bytes.len() - off,
                )
            };
            if n <= 0 {
                return;
            }
            off += n as usize;
        }
    }

    fn fault_addr(info: *mut libc::siginfo_t) -> usize {
        if info.is_null() {
            return 0;
        }
        // SAFETY: the kernel passes a valid siginfo_t to SA_SIGINFO handlers.
        #[cfg(target_os = "linux")]
        return unsafe { (*info).si_addr() as usize };
        #[cfg(not(target_os = "linux"))]
        return unsafe { (*info).si_addr as usize };
    }

    /// Restore the default disposition for `sig`, unblock it (it is blocked
    /// while its own handler runs), and re-raise so the process dies with the
    /// original signal.
    fn reset_and_reraise(sig: libc::c_int) -> ! {
        // SAFETY: sigaction/pthread_sigmask/raise are async-signal-safe;
        // structs are fully initialized before use.
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = libc::SIG_DFL;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(sig, &sa, std::ptr::null_mut());

            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, sig);
            libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());

            libc::raise(sig);
            // Only reached if delivery was somehow deferred; exit with the
            // conventional 128+signal code rather than returning into faulted
            // state.
            libc::_exit(128 + sig);
        }
    }

    extern "C" fn handler(sig: libc::c_int, info: *mut libc::siginfo_t, _ctx: *mut libc::c_void) {
        if HANDLING.swap(true, Ordering::SeqCst) {
            reset_and_reraise(sig);
        }

        write_stderr(b"\naspect-cli: fatal signal: ");
        write_stderr(signal_name(sig).as_bytes());
        write_stderr(b"\naspect-cli: collecting backtrace (best effort)...\n");

        // Allocating from here — see the module docstring for why that trade
        // is acceptable inside a dying process.
        let thread = std::thread::current();
        write_stderr(
            format_report(fault_addr(info), thread.name().unwrap_or("<unnamed>")).as_bytes(),
        );

        reset_and_reraise(sig);
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn signal_name_maps_known_and_unknown() {
            assert_eq!(signal_name(libc::SIGSEGV), "SIGSEGV (segmentation fault)");
            assert_eq!(signal_name(libc::SIGABRT), "SIGABRT (abort)");
            assert_eq!(signal_name(libc::SIGILL), "SIGILL (illegal instruction)");
            assert_eq!(signal_name(9999), "unknown fatal signal");
        }

        #[test]
        fn every_fatal_signal_has_a_name() {
            for &(sig, label) in FATAL_SIGNALS {
                assert_eq!(signal_name(sig), label);
            }
        }

        #[test]
        fn format_report_includes_address_thread_and_pointer() {
            let report = format_report(0xdead_beef, "worker-3");
            assert!(report.contains("fault address: 0xdeadbeef"), "{report}");
            assert!(report.contains("thread: worker-3"), "{report}");
            assert!(report.contains("backtrace:"), "{report}");
            assert!(
                report.contains("github.com/aspect-build/aspect-cli/issues"),
                "{report}"
            );
        }

        #[test]
        fn opt_out_detects_nonempty_only() {
            // SAFETY: single-threaded test; no other thread reads the env here.
            unsafe {
                std::env::remove_var(OPT_OUT_ENV);
                assert!(!opt_out());
                std::env::set_var(OPT_OUT_ENV, "");
                assert!(!opt_out());
                std::env::set_var(OPT_OUT_ENV, "1");
                assert!(opt_out());
                std::env::remove_var(OPT_OUT_ENV);
            }
        }
    }
}
