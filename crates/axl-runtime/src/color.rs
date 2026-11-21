//! Whether hand-rendered output may carry ANSI color.
//!
//! Clap strips the escapes baked into our `--help` templates when it renders to
//! a non-terminal, so help output is already safe. The surfaces that print their
//! own layout (`aspect feature`, the console lines around a task, and anything
//! else assembling SGR sequences by hand) have no such protection and would
//! otherwise emit raw escapes into a pipe — a log file, a `| grep`, or an AI
//! agent capturing output.
//!
//! A non-empty `NO_COLOR` (<https://no-color.org>) turns color off on every
//! surface, CI hosts whose logs render ANSI included.
//!
//! AXL asks the same rules through the std streams: `std.io.stderr.color` and
//! `std.io.stdout.color` report the [`StreamColor`] the run resolved.

use std::ffi::OsString;
use std::io::IsTerminal;

/// Whether each of the process's console streams may carry ANSI color,
/// resolved once per run and reported to AXL as `std.io.<stream>.color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamColor {
    pub stdout: bool,
    pub stderr: bool,
}

impl StreamColor {
    /// [`stdout_supports_color`] and [`console_supports_color`].
    pub fn of_process() -> Self {
        Self {
            stdout: stdout_supports_color(),
            stderr: console_supports_color(),
        }
    }

    /// The same rules over a snapshot: whether each stream is a terminal, and
    /// `var` reading one environment variable.
    pub fn from_snapshot(
        stdout_tty: bool,
        stderr_tty: bool,
        var: impl Fn(&str) -> Option<OsString>,
    ) -> Self {
        Self {
            stdout: stdout_color_from(stdout_tty, &var),
            stderr: console_color_from(stderr_tty, &var),
        }
    }
}

/// Whether stdout may carry ANSI color: [`stdout_color_from`] over the process.
pub fn stdout_supports_color() -> bool {
    stdout_color_from(std::io::stdout().is_terminal(), process_env)
}

/// Whether the lines the runtime prints to stderr (the console slots, `WARNING:`
/// and other severity prefixes, the grey Bazel trace line) may carry ANSI
/// color: [`console_color_from`] over the process. The console resolves it once
/// for every slot; AXL reads it as `std.io.stderr.color`.
pub fn console_supports_color() -> bool {
    console_color_from(std::io::stderr().is_terminal(), process_env)
}

/// The stdout rule over a snapshot: only on an interactive stdout, and never
/// under a non-empty `NO_COLOR`. `var` reads one environment variable.
fn stdout_color_from(stdout_tty: bool, var: impl Fn(&str) -> Option<OsString>) -> bool {
    stdout_tty && !no_color(&var)
}

/// The stderr rule over a snapshot: on an interactive stderr, or on a CI host
/// whose log viewer renders ANSI ([`crate::ci::ci_renders_ansi`]), and never
/// under a non-empty `NO_COLOR`. `var` reads one environment variable.
fn console_color_from(stderr_tty: bool, var: impl Fn(&str) -> Option<OsString>) -> bool {
    let renders = stderr_tty || crate::ci::renders_ansi_from(|name| non_empty(var(name)));
    renders && !no_color(&var)
}

/// Whether `NO_COLOR` asks for no color: set to a non-empty value, as the
/// no-color.org spec reads it.
fn no_color(var: &impl Fn(&str) -> Option<OsString>) -> bool {
    non_empty(var("NO_COLOR"))
}

fn non_empty(value: Option<OsString>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

fn process_env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: Vec<(String, OsString)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    #[test]
    fn stdout_color_only_on_a_tty_without_no_color() {
        assert!(stdout_color_from(true, env(&[])));
        assert!(!stdout_color_from(false, env(&[])));
        assert!(!stdout_color_from(
            false,
            env(&[("GITHUB_ACTIONS", "true")])
        ));
        assert!(!stdout_color_from(true, env(&[("NO_COLOR", "1")])));
        assert!(stdout_color_from(true, env(&[("NO_COLOR", "")])));
    }

    #[test]
    fn console_color_on_a_tty_or_an_ansi_rendering_ci_host() {
        assert!(console_color_from(true, env(&[])));
        assert!(!console_color_from(false, env(&[])));
        assert!(console_color_from(false, env(&[("BUILDKITE", "true")])));
        assert!(!console_color_from(false, env(&[("CI", "1")])));
        assert!(!console_color_from(false, env(&[("GITHUB_ACTIONS", "")])));
    }

    #[test]
    fn a_snapshot_resolves_each_stream_by_its_own_rule() {
        let on_ci = StreamColor::from_snapshot(false, false, env(&[("GITHUB_ACTIONS", "true")]));
        assert_eq!(
            on_ci,
            StreamColor {
                stdout: false,
                stderr: true
            }
        );
        let tty = StreamColor::from_snapshot(true, true, env(&[("NO_COLOR", "1")]));
        assert_eq!(
            tty,
            StreamColor {
                stdout: false,
                stderr: false
            }
        );
    }

    #[test]
    fn a_non_empty_no_color_wins_on_ci_too() {
        assert!(!console_color_from(true, env(&[("NO_COLOR", "1")])));
        assert!(!console_color_from(
            false,
            env(&[("GITHUB_ACTIONS", "true"), ("NO_COLOR", "0")])
        ));
        assert!(console_color_from(true, env(&[("NO_COLOR", "")])));
        assert!(console_color_from(
            false,
            env(&[("GITLAB_CI", "true"), ("NO_COLOR", "")])
        ));
    }
}
