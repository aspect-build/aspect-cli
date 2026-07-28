//! Fatal-signal crash reporter.
//!
//! Installs handlers for the fatal signals (SIGSEGV, SIGBUS, SIGILL, SIGFPE,
//! SIGABRT) that print the signal name, fault address, and a raw-address
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
//! Async-signal-safety: everything the handler does is signal-safe. Output
//! goes through raw `write(2)`; integers are formatted in a stack buffer; the
//! stack walk uses `backtrace::trace_unsynchronized` (no allocator lock) and
//! emits raw instruction pointers only — it does NOT symbolize, because
//! resolving symbols allocates and, on static-musl release builds (the case
//! this handler exists for), faults inside the handler. A re-entrancy flag
//! turns any fault inside the handler into an immediate default-action death.
//! Handlers run on the alternate signal stack (`SA_ONSTACK`) std installs per
//! thread, so stack-overflow SIGSEGVs are reported rather than double-faulting.
//!
//! Resolving the report: release binaries are position-independent, so each
//! code address is printed as `<runtime> (+<static offset>)` where the offset
//! already has the ASLR load bias removed. Release builds keep their symbols
//! (see `bazel/rust/defs.bzl`), so the offset resolves to a function and
//! file:line directly against the binary: `addr2line -fe aspect-cli <offset>`.
//!
//! `ASPECT_NO_CRASH_HANDLER` (any non-empty value) skips installation.

/// Install the fatal-signal handlers. Call first thing in `main`, before any
/// runtime machinery, so the reporter covers everything after it. No-op on
/// non-unix platforms and under `ASPECT_NO_CRASH_HANDLER`.
pub fn install() {
    #[cfg(unix)]
    unix::install();
}

/// Test hook: crash the process immediately per `ASPECT_INTERNAL_TEST_CRASH`
/// (`segv`, `abort`, `segv-thread`, or `stackoverflow-thread`), otherwise a
/// no-op. Lets the end-to-end tests drive a real crash — on the main thread, a
/// spawned thread, or via stack overflow — through a spawned binary. Kept
/// separate from [`install`] so a test can exercise the
/// `ASPECT_NO_CRASH_HANDLER` opt-out (handler skipped) while still crashing.
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
            // SAFETY: same null write, but on a spawned thread (models the
            // Starlark task running on a tokio blocking/worker thread).
            Ok("segv-thread") => {
                std::thread::spawn(|| unsafe { std::ptr::null_mut::<u8>().write_volatile(1) })
                    .join()
                    .ok();
            }
            // Unbounded recursion → stack-overflow SIGSEGV, on a spawned thread.
            Ok("stackoverflow-thread") => {
                std::thread::spawn(|| {
                    #[allow(unconditional_recursion)] // intentional overflow
                    fn recurse(x: u64) -> u64 {
                        let buf = [x; 1024];
                        recurse(std::hint::black_box(buf[0]).wrapping_add(1))
                    }
                    std::hint::black_box(recurse(0));
                })
                .join()
                .ok();
            }
            // Crash during process teardown, after `main` has returned: the
            // libc exit path runs atexit handlers and static destructors, and
            // a fault there must still be reported.
            Ok("segv-atexit") => {
                extern "C" fn boom() {
                    // SAFETY: intentional null write to raise SIGSEGV.
                    unsafe { std::ptr::null_mut::<u8>().write_volatile(1) }
                }
                // SAFETY: registering an atexit handler.
                unsafe { libc::atexit(boom) };
            }
            // Crash on a detached thread while the main thread is exiting —
            // the shape of a background reader still running at teardown.
            Ok("segv-detached-at-exit") => {
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    // SAFETY: intentional null write to raise SIGSEGV.
                    unsafe { std::ptr::null_mut::<u8>().write_volatile(1) }
                });
            }
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

    /// `0x`-prefixed hex of `value`, formatted into `buf` (no allocation). The
    /// returned slice borrows `buf`. `buf` must be at least 18 bytes (`0x` + 16
    /// hex digits, the max for a 64-bit usize). Split from [`write_hex`] so the
    /// formatting is unit-testable without capturing stderr.
    fn format_hex(value: usize, buf: &mut [u8; 18]) -> &[u8] {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut v = value;
        let mut i = buf.len();
        loop {
            i -= 1;
            buf[i] = HEX[v & 0xf];
            v >>= 4;
            if v == 0 {
                break;
            }
        }
        i -= 1;
        buf[i] = b'x';
        i -= 1;
        buf[i] = b'0';
        &buf[i..]
    }

    /// Write `value` as `0x`-prefixed hex to stderr. Async-signal-safe.
    fn write_hex(value: usize) {
        let mut buf = [0u8; 18];
        write_stderr(format_hex(value, &mut buf));
    }

    /// The main executable's load bias (the runtime base a position-independent
    /// executable was mapped at). Subtract it from a runtime address to get the
    /// static, ASLR-independent offset that resolves against the on-disk binary
    /// (`addr2line -e aspect-cli <offset>`). Returns 0 on non-Linux or if the
    /// bias can't be determined — in which case the printed offset equals the
    /// raw address.
    fn load_bias() -> usize {
        #[cfg(target_os = "linux")]
        {
            // The first object `dl_iterate_phdr` reports is the main
            // executable; its `dlpi_addr` is the load bias. Stop after it.
            extern "C" fn cb(
                info: *mut libc::dl_phdr_info,
                _size: libc::size_t,
                data: *mut libc::c_void,
            ) -> libc::c_int {
                // SAFETY: `info` is a valid dl_phdr_info for the callback's
                // lifetime; `data` is the &mut usize we passed in.
                unsafe {
                    *(data as *mut usize) = (*info).dlpi_addr as usize;
                }
                1 // non-zero: visit only the first (main executable) entry
            }
            let mut bias: usize = 0;
            // SAFETY: dl_iterate_phdr with a callback that only writes through
            // the provided pointer; no allocation.
            unsafe {
                libc::dl_iterate_phdr(Some(cb), (&mut bias) as *mut usize as *mut libc::c_void);
            }
            bias
        }
        #[cfg(not(target_os = "linux"))]
        {
            0
        }
    }

    /// Write a runtime code address as `<abs> (+<offset>)`, where `offset` is
    /// `abs - bias` — the static offset that resolves against the on-disk
    /// binary regardless of ASLR. When `bias` is 0 the offset equals `abs`.
    fn write_addr(abs: usize, bias: usize) {
        write_hex(abs);
        write_stderr(b" (+");
        write_hex(abs.wrapping_sub(bias));
        write_stderr(b")");
    }

    /// Walk the current call stack and write each frame's instruction pointer
    /// to stderr as `<abs> (+<offset>)` (see [`write_addr`]), one per line.
    /// Signal-safe: it neither allocates nor symbolizes (both fault inside a
    /// signal handler on static-musl builds — the very case this handler exists
    /// for).
    fn write_raw_backtrace(bias: usize) {
        // SAFETY: single-shot within a dying process; `trace_unsynchronized`
        // avoids the global lock `trace` takes (which could deadlock if the
        // crash happened while that lock was held). The callback only writes
        // to stderr.
        unsafe {
            backtrace::trace_unsynchronized(|frame| {
                write_stderr(b"  ");
                write_addr(frame.ip() as usize, bias);
                write_stderr(b"\n");
                true
            });
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

    /// The program counter at the moment of the fault, read from the signal's
    /// `ucontext`. This is the address to resolve first — it is where the crash
    /// actually happened, independent of whether the stack unwinds. Returns 0
    /// when unavailable (null context, or an arch/OS we don't decode).
    ///
    /// A static-musl signal handler often can't unwind *through* the kernel
    /// signal frame, so [`write_raw_backtrace`] alone may yield only the
    /// handler's own frame; the ucontext PC is the reliable crash location.
    fn crash_pc(ctx: *mut libc::c_void) -> usize {
        if ctx.is_null() {
            return 0;
        }
        // SAFETY: for SA_SIGINFO handlers the kernel passes a valid
        // ucontext_t*; we read the saved PC register for the target arch.
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        unsafe {
            let uc = ctx as *mut libc::ucontext_t;
            return (*uc).uc_mcontext.gregs[libc::REG_RIP as usize] as usize;
        }
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        unsafe {
            let uc = ctx as *mut libc::ucontext_t;
            return (*uc).uc_mcontext.pc as usize;
        }
        #[allow(unreachable_code)]
        {
            let _ = ctx;
            0
        }
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

    extern "C" fn handler(sig: libc::c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
        if HANDLING.swap(true, Ordering::SeqCst) {
            reset_and_reraise(sig);
        }

        let bias = load_bias();

        write_stderr(b"\naspect-cli: fatal signal: ");
        write_stderr(signal_name(sig).as_bytes());
        write_stderr(b"\nfault address: ");
        write_hex(fault_addr(info));
        write_stderr(b"\ncrash pc: ");
        write_addr(crash_pc(ctx), bias);
        write_stderr(
            b"\ncode addresses print as `<runtime> (+<static offset>)`; resolve the \
              static offset against this binary:\n  \
              `addr2line -fe aspect-cli <offset>`.\ncrash pc is the fault site; the \
              frames below may be truncated to the handler frame on static-musl builds:\n",
        );
        write_raw_backtrace(bias);
        write_stderr(
            b"aspect-cli crashed; please report this at \
              https://github.com/aspect-build/aspect-cli/issues including the output above.\n",
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
        fn format_hex_matches_std_and_handles_edges() {
            let mut buf = [0u8; 18];
            assert_eq!(format_hex(0, &mut buf), b"0x0");
            assert_eq!(format_hex(0xdead_beef, &mut buf), b"0xdeadbeef");
            assert_eq!(format_hex(usize::MAX, &mut buf), b"0xffffffffffffffff");
            for v in [1usize, 0xf, 0x10, 0x1234, 0x8000_0000_0000_0000] {
                assert_eq!(format_hex(v, &mut buf), format!("{v:#x}").as_bytes());
            }
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
