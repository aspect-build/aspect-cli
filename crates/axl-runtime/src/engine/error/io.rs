//! `std.io.Error`: what a failed I/O operation raises.

use std::io;

use crate::engine::error::native_error;

native_error! {
    /// A failed I/O operation: what `std.net` raises.
    ///
    /// `std.io.Error` derives from `error`, so `isinstance(e, error)` holds
    /// too, and it adds one field, `kind`, saying what went wrong. `message`
    /// is the operating system's or library's description.
    ///
    /// Catch it by type, so a bug in your own code is not mistaken for a
    /// network failure, or call a method's `try_` twin, which returns the same
    /// `(err, value)` pair:
    ///
    /// ```starlark
    /// err, s = catch(ctx.std.net.tcp.connect, addr, timeout_ms = 2000, types = [std.io.Error])
    /// err, s = ctx.std.net.tcp.try_connect(addr, timeout_ms = 2000)
    /// if err and err.kind == "connection_refused":
    ///     ...
    /// ```
    pub(crate) struct IoError = "std.io.Error" {
        /// What went wrong: the name of Rust's `std::io::ErrorKind` in snake
        /// case, such as `"timed_out"`, `"connection_refused"`, `"not_found"`,
        /// `"unexpected_eof"`, `"broken_pipe"` or `"invalid_data"`; `"other"`
        /// for anything else. A read or write that runs out its timeout is
        /// always `"timed_out"`, on every platform.
        kind: String,
    }
}

impl From<io::Error> for IoError {
    fn from(err: io::Error) -> Self {
        Self {
            message: err.to_string(),
            kind: kind_name(err.kind()).to_owned(),
        }
    }
}

/// `kind`'s name as `std.io.Error.kind` spells it: Rust's `ErrorKind` variant
/// in snake case. A kind Rust adds later, or one not yet stable, is `other`.
fn kind_name(kind: io::ErrorKind) -> &'static str {
    use io::ErrorKind::*;
    match kind {
        NotFound => "not_found",
        PermissionDenied => "permission_denied",
        ConnectionRefused => "connection_refused",
        ConnectionReset => "connection_reset",
        HostUnreachable => "host_unreachable",
        NetworkUnreachable => "network_unreachable",
        ConnectionAborted => "connection_aborted",
        NotConnected => "not_connected",
        AddrInUse => "addr_in_use",
        AddrNotAvailable => "addr_not_available",
        NetworkDown => "network_down",
        BrokenPipe => "broken_pipe",
        AlreadyExists => "already_exists",
        WouldBlock => "would_block",
        NotADirectory => "not_a_directory",
        IsADirectory => "is_a_directory",
        DirectoryNotEmpty => "directory_not_empty",
        ReadOnlyFilesystem => "read_only_filesystem",
        StaleNetworkFileHandle => "stale_network_file_handle",
        InvalidInput => "invalid_input",
        InvalidData => "invalid_data",
        TimedOut => "timed_out",
        WriteZero => "write_zero",
        StorageFull => "storage_full",
        NotSeekable => "not_seekable",
        QuotaExceeded => "quota_exceeded",
        FileTooLarge => "file_too_large",
        ResourceBusy => "resource_busy",
        ExecutableFileBusy => "executable_file_busy",
        Deadlock => "deadlock",
        CrossesDevices => "crosses_devices",
        TooManyLinks => "too_many_links",
        InvalidFilename => "invalid_filename",
        ArgumentListTooLong => "argument_list_too_long",
        Interrupted => "interrupted",
        Unsupported => "unsupported",
        UnexpectedEof => "unexpected_eof",
        OutOfMemory => "out_of_memory",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_rust_names_in_snake_case() {
        assert_eq!(kind_name(io::ErrorKind::TimedOut), "timed_out");
        assert_eq!(
            kind_name(io::ErrorKind::ConnectionRefused),
            "connection_refused"
        );
        assert_eq!(kind_name(io::ErrorKind::NotFound), "not_found");
        assert_eq!(kind_name(io::ErrorKind::Other), "other");
    }
}
