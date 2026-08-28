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
