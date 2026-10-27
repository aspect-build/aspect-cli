//! Building bazel command lines and reading what comes back.

pub mod flags;
pub mod query;
pub mod run;

/// Bazel's `stderr` rendered as a trailing detail for a spawn-failure error,
/// on its own line. Empty when Bazel wrote nothing, so a command that died
/// without diagnostics still yields a clean exit-code message.
///
/// Bazel reuses a handful of exit codes across unrelated causes (`2` covers
/// every command-line error — unknown startup option, unrecognized flag,
/// unknown `info` key, running outside a workspace), so the code alone rarely
/// identifies the failure.
pub fn stderr_detail(stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        String::new()
    } else {
        format!("\n{stderr}")
    }
}

/// The version `INFO:` text. `version` is `None` for a non-release build (see
/// [`crate::server::info::parse_release`]), which notes the assume-latest
/// behavior.
pub fn version_line(version: Option<&semver::Version>) -> String {
    match version {
        Some(v) => format!("Bazel {v}"),
        None => "Bazel development version (version-conditional flags assume latest)".to_string(),
    }
}

/// Render `cmd` as a space-joined `program arg…` line for display. Read back
/// from the fully assembled `Command`, so it shows the full argument set
/// passed to Bazel (including the internal BES/execlog flags).
///
/// Secrets (env-var values, request headers, URL credentials) are redacted via
/// [`crate::bes::redact::redact_command_args`] — the same rules the BES
/// sink redaction uses — since this line is printed to CI logs by default.
/// Args are space-joined for readability, not shell-quoted: a value with a
/// space (or a `<REDACTED>` placeholder) is not guaranteed copy-paste-safe.
pub fn render(cmd: &std::process::Command) -> String {
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let redacted = crate::bes::redact::redact_command_args(args.iter().map(String::as_str));
    std::iter::once(cmd.get_program().to_string_lossy().into_owned())
        .chain(redacted)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{render, stderr_detail, version_line};

    #[test]
    fn version_line_text() {
        assert_eq!(
            version_line(Some(&semver::Version::new(9, 0, 1))),
            "Bazel 9.0.1"
        );
        assert_eq!(
            version_line(None),
            "Bazel development version (version-conditional flags assume latest)"
        );
    }

    #[test]
    fn render_joins_program_and_args() {
        let mut cmd = std::process::Command::new("bazel");
        cmd.args(["--bazelrc=/dev/null", "build", "--", "//foo:bar"]);
        assert_eq!(render(&cmd), "bazel --bazelrc=/dev/null build -- //foo:bar");
    }

    #[test]
    fn render_redacts_env_secrets() {
        // The redaction rules themselves are covered in `bes::redact`'s own
        // tests; this asserts the wiring (secret env values are hidden, the
        // command shape is preserved).
        let mut cmd = std::process::Command::new("bazel");
        cmd.args(["build", "--action_env=DB_PASSWORD=hunter2", "//foo"]);
        let rendered = render(&cmd);
        assert!(rendered.starts_with("bazel build --action_env=DB_PASSWORD="));
        assert!(rendered.ends_with(" //foo"));
        assert!(!rendered.contains("hunter2"), "secret leaked: {rendered}");
    }

    #[test]
    fn stderr_detail_is_a_trailing_line_when_present() {
        assert_eq!(stderr_detail("ERROR: boom\n"), "\nERROR: boom");
    }

    #[test]
    fn stderr_detail_is_empty_without_diagnostics() {
        assert_eq!(stderr_detail(""), "");
        assert_eq!(stderr_detail("  \n\t "), "");
    }
}
