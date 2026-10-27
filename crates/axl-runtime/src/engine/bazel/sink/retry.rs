//! The runner's environment override for the gRPC BES sink's replay buffer.
//!
//! The retry machinery itself is [`bazel::bes::sink::retry`]; reading the
//! environment and warning the user about a bad value is the runtime's job.

use bazel::bes::sink::retry::{DEFAULT_RETRY_MAX_BUFFER_BYTES, parse_byte_size};

/// Environment override for the replay buffer's byte budget, letting a runner
/// size it to its own memory without threading a flag through every task.
/// Accepts a plain byte count or a size suffix — see [`parse_byte_size`].
///
/// The `CLI` in the name distinguishes this from Bazel's own BES uploader
/// (`--bes_backend`), whose buffering it does not affect.
pub const RETRY_MAX_BUFFER_BYTES_ENV: &str = "ASPECT_CLI_BES_RETRY_MAX_BUFFER_BYTES";

/// The replay buffer's default byte budget, honoring
/// [`RETRY_MAX_BUFFER_BYTES_ENV`].
///
/// Read once per process and cached, so two sinks in one build cannot disagree
/// if the environment changes mid-run.
pub fn default_retry_max_buffer_bytes() -> usize {
    static RESOLVED: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *RESOLVED.get_or_init(|| {
        let (bytes, warning) = resolve_buffer_bytes_override(
            std::env::var(RETRY_MAX_BUFFER_BYTES_ENV).ok().as_deref(),
        );
        if let Some(w) = warning {
            crate::diag::warn(&w);
        }
        bytes
    })
}

/// Resolve the override, returning the budget to use and any warning to emit.
/// Separate from [`default_retry_max_buffer_bytes`] so the rules are testable
/// without touching process-wide state.
///
/// Unset or empty is silent — the common case. A malformed or zero value warns
/// and falls back to the built-in default: BES upload is best-effort, so a
/// typo'd tuning knob should not cost a CI run.
fn resolve_buffer_bytes_override(raw: Option<&str>) -> (usize, Option<String>) {
    let Some(raw) = raw else {
        return (DEFAULT_RETRY_MAX_BUFFER_BYTES, None);
    };
    if raw.trim().is_empty() {
        return (DEFAULT_RETRY_MAX_BUFFER_BYTES, None);
    }
    match parse_byte_size(raw) {
        Ok(0) => (
            DEFAULT_RETRY_MAX_BUFFER_BYTES,
            Some(format!(
                "{RETRY_MAX_BUFFER_BYTES_ENV}={raw} must be > 0; \
                 using the default of {DEFAULT_RETRY_MAX_BUFFER_BYTES} bytes"
            )),
        ),
        Ok(n) => (n, None),
        Err(e) => (
            DEFAULT_RETRY_MAX_BUFFER_BYTES,
            Some(format!(
                "{RETRY_MAX_BUFFER_BYTES_ENV}: {e}; \
                 using the default of {DEFAULT_RETRY_MAX_BUFFER_BYTES} bytes"
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end check that the resolver reads the real environment variable
    /// under its documented name — the unit tests below exercise the fallback
    /// rules but would keep passing if `RETRY_MAX_BUFFER_BYTES_ENV` were
    /// misspelled or `default_retry_max_buffer_bytes` stopped consulting it.
    ///
    /// Cannot call `default_retry_max_buffer_bytes` itself: it caches in a
    /// process-wide `OnceLock`, so the value it returns depends on whichever
    /// test touched it first. This drives the same path with an explicit read.
    #[test]
    fn env_var_name_is_read_from_the_environment() {
        // SAFETY: single-threaded within this test, and the value is removed
        // before returning so no other test observes it.
        unsafe { std::env::set_var(RETRY_MAX_BUFFER_BYTES_ENV, "7MB") };
        let raw = std::env::var(RETRY_MAX_BUFFER_BYTES_ENV).ok();
        let (bytes, warning) = resolve_buffer_bytes_override(raw.as_deref());
        unsafe { std::env::remove_var(RETRY_MAX_BUFFER_BYTES_ENV) };

        assert_eq!(bytes, 7 * 1024 * 1024);
        assert!(warning.is_none());
        assert_eq!(
            RETRY_MAX_BUFFER_BYTES_ENV, "ASPECT_CLI_BES_RETRY_MAX_BUFFER_BYTES",
            "the documented variable name is part of the public interface"
        );
    }

    /// A usable override is taken verbatim, with no warning.
    #[test]
    fn env_override_applies_when_valid() {
        let (bytes, warning) = resolve_buffer_bytes_override(Some("512MB"));
        assert_eq!(bytes, 512 * 1024 * 1024);
        assert!(warning.is_none(), "a valid override should be silent");

        let (bytes, warning) = resolve_buffer_bytes_override(Some("1048576"));
        assert_eq!(bytes, 1024 * 1024);
        assert!(warning.is_none());
    }

    /// Unset or empty is the common case and must not warn.
    #[test]
    fn env_override_absent_is_silent_default() {
        for raw in [None, Some(""), Some("   ")] {
            let (bytes, warning) = resolve_buffer_bytes_override(raw);
            assert_eq!(bytes, DEFAULT_RETRY_MAX_BUFFER_BYTES, "{raw:?}");
            assert!(warning.is_none(), "{raw:?} should not warn");
        }
    }

    /// A malformed or zero override falls back to the default and warns — a
    /// typo'd tuning knob must never break a build or silently disable the
    /// buffer.
    #[test]
    fn env_override_invalid_warns_and_falls_back() {
        for bad in ["garbage", "12TB", "1.5MB", "-1", "0", "0MB"] {
            let (bytes, warning) = resolve_buffer_bytes_override(Some(bad));
            assert_eq!(
                bytes, DEFAULT_RETRY_MAX_BUFFER_BYTES,
                "{bad:?} should fall back to the default"
            );
            let warning = warning.unwrap_or_else(|| panic!("{bad:?} should warn"));
            assert!(
                warning.contains(RETRY_MAX_BUFFER_BYTES_ENV),
                "warning should name the env var: {warning}"
            );
        }
    }
}
