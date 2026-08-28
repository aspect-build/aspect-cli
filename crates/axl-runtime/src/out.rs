//! Console writes that survive a closed pipe.
//!
//! `println!` / `eprintln!` panic when the write fails, and a closed reader is
//! the common case: `aspect build … | head`, or a CI assertion piping into
//! `grep -q`, closes the pipe as soon as it has what it wants. The panic aborts
//! the process mid-task, so nothing runs the task's terminal update and its
//! GitHub check run is left "running" until the API sweeper finalizes it as
//! DISCONNECTED.
//!
//! These macros discard the write error instead. Output past the close is lost
//! either way — nobody is reading it — but the process finishes its work,
//! reports its real result, and exits with its real code.
//!
//! For human-facing console output only. Anything a *program* consumes — a
//! credential-helper response, a machine-readable dump — must propagate the
//! failure rather than report success on output nobody received.

/// `println!` that ignores a failed write (notably a closed pipe).
#[macro_export]
macro_rules! outln {
    () => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout());
    }};
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

/// `print!` that ignores a failed write (notably a closed pipe).
#[macro_export]
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = write!(std::io::stdout(), $($arg)*);
    }};
}

/// `eprintln!` that ignores a failed write (notably a closed pipe).
#[macro_export]
macro_rules! errln {
    () => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr());
    }};
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

/// AXL `print()` that survives a closed stderr.
///
/// Starlark's default handler is a bare `eprintln!`, which panics when the
/// write fails — so any `print()` from AXL takes the process down as soon as a
/// reader leaves (`aspect build … | head`). The panic aborts mid-task, so the
/// task never runs its terminal update and its status-surface entry is
/// stranded showing "running".
///
/// Install with `eval.set_print_handler(&TOLERANT_PRINT_HANDLER)`. Discarding
/// the write matches `outln!`/`errln!`: nobody is reading the output, and
/// finishing the task matters more than the line nobody sees.
pub struct TolerantPrintHandler;

impl starlark::PrintHandler for TolerantPrintHandler {
    fn println(&self, text: &str) -> starlark::Result<()> {
        errln!("{text}");
        Ok(())
    }
}

/// Shared instance — the handler is stateless.
pub static TOLERANT_PRINT_HANDLER: TolerantPrintHandler = TolerantPrintHandler;
