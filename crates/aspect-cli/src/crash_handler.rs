//! Fatal-signal crash reporter.
//!
//! Installs handlers for the fatal signals (SIGSEGV, SIGBUS, SIGILL, SIGFPE,
//! SIGABRT) that print the signal name, fault address, and a best-effort
//! backtrace to stderr, then re-deliver the signal with its default
//! disposition so the process still dies with the original signal and CI
//! harnesses observe the same exit status they do today.
//!
//! Motivation: a native crash on a CI runner otherwise surfaces as a bare
//! `Received "segmentation fault" signal` line from the CI harness — nothing
//! to debug from, and kernel logs / core dumps on ephemeral runners are
//! usually gone before anyone can look. With this handler the crash report
//! lands in the task log itself.
//!
//! Async-signal-safety: capturing and symbolizing a backtrace allocates,
//! which is formally undefined inside a signal handler (the crashing thread
//! may hold the allocator lock). This is a deliberate best-effort trade,
//! same as rustc's own fatal-signal handler: the process is already dying,
//! a marker naming the signal is written with raw `write(2)` *before* the
//! allocating section so even a wedged backtrace capture identifies the
//! signal, and a re-entrancy guard turns a fault inside the handler into an
//! immediate default-action death. Handlers run on the alternate signal
//! stack Rust's runtime installs per thread (`SA_ONSTACK`), so stack-overflow
//! SIGSEGVs are reported too — the backtrace then shows the overflowing
//! recursion in place of std's one-line overflow message.
//!
//! `ASPECT_NO_CRASH_HANDLER` (any non-empty value) skips installation.

/// Install the fatal-signal handlers. Call first thing in `main`, before
/// any runtime machinery, so the reporter covers everything after it.
/// No-op on non-unix platforms and under `ASPECT_NO_CRASH_HANDLER`.
pub fn install() {
    #[cfg(unix)]
    unix::install();
}

/// Test hook: crash the process immediately when `ASPECT_INTERNAL_TEST_CRASH`
/// is set (`segv` or `abort`), so integration tests can exercise the handler
/// end-to-end through a real spawned binary. Called from `main` right after
/// [`install`] — deliberately outside it, so the `ASPECT_NO_CRASH_HANDLER`
/// opt-out path is testable (handler skipped, crash still triggered).
#[doc(hidden)]
pub fn trigger_test_crash() {
    #[cfg(unix)]
    unix::trigger_test_crash();
}

#[cfg(unix)]
mod unix {
    use std::sync::atomic::{AtomicBool, Ordering};

    const FATAL_SIGNALS: &[(libc::c_int, &str)] = &[
        (libc::SIGSEGV, "SIGSEGV (segmentation fault)"),
        (libc::SIGBUS, "SIGBUS (bus error)"),
        (libc::SIGILL, "SIGILL (illegal instruction)"),
        (libc::SIGFPE, "SIGFPE (arithmetic exception)"),
        (libc::SIGABRT, "SIGABRT (abort)"),
    ];

    /// True once a handler is running; a second entry (fault inside the
    /// handler, or a simultaneous crash on another thread) skips the report
    /// and goes straight to the default action.
    static HANDLING: AtomicBool = AtomicBool::new(false);

    pub(super) fn install() {
        if std::env::var_os("ASPECT_NO_CRASH_HANDLER").is_some_and(|v| !v.is_empty()) {
            return;
        }
        let f: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) = handler;
        for &(sig, _) in FATAL_SIGNALS {
            // SAFETY: standard handler registration; `sa` is fully
            // initialized before use.
            unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = f as usize;
                sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut sa.sa_mask);
                libc::sigaction(sig, &sa, std::ptr::null_mut());
            }
        }
    }

    pub(super) fn trigger_test_crash() {
        match std::env::var("ASPECT_INTERNAL_TEST_CRASH").as_deref() {
            // SAFETY: intentional null-pointer write to raise SIGSEGV.
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

    /// Write directly to stderr with `write(2)`, ignoring errors.
    /// Async-signal-safe.
    fn write_stderr(bytes: &[u8]) {
        let mut off = 0;
        while off < bytes.len() {
            // SAFETY: in-bounds pointer/length into `bytes` for fd 2.
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

    /// Restore the default disposition, unblock the signal (it is blocked
    /// while its own handler runs), and re-raise — the process dies with the
    /// original signal, so the exit status CI harnesses see is unchanged.
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
            // Unreachable unless delivery was somehow deferred; exit with
            // the conventional 128+signal code rather than continuing.
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

        // Allocating from here on — see the module docstring for why that
        // is an acceptable trade inside a crash handler.
        let thread = std::thread::current();
        let report = format!(
            "fault address: {:#x}\nthread: {}\nbacktrace:\n{}\naspect-cli crashed; please report this at https://github.com/aspect-build/aspect-cli/issues including the output above.\n",
            fault_addr(info),
            thread.name().unwrap_or("<unnamed>"),
            std::backtrace::Backtrace::force_capture(),
        );
        write_stderr(report.as_bytes());

        reset_and_reraise(sig);
    }
}
