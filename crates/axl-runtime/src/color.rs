//! Whether hand-rendered output may carry ANSI color.
//!
//! Clap decides for itself whether `--help` and its errors carry color. The
//! surfaces that print their own layout (`aspect feature`, the console lines
//! around a task, and anything else assembling SGR sequences by hand) ask here,
//! so they must agree with clap: the rules below are clap's (anstream's), in
//! its order, with one addition for stderr on CI hosts whose logs render ANSI.
//!
//! 1. A non-empty `NO_COLOR` (<https://no-color.org>) turns color off.
//! 2. A non-empty `CLICOLOR_FORCE` (<https://bixense.com/clicolors>) turns it on.
//! 3. `CLICOLOR=0` turns it off.
//! 4. A terminal gets color when `TERM` is set and not `dumb`, or when
//!    `CLICOLOR` or `CI` is set. Stderr also gets it on a CI host whose log
//!    viewer renders ANSI ([`crate::ci::renders_ansi`]). Anything else, such as
//!    a log file, a `| grep` or an AI agent capturing output, stays plain.
//!
//! The process's answer is resolved once, on first use, so every surface agrees
//! for the whole run even if a task later edits its environment. AXL reads it
//! through the std streams: `std.io.stderr.color` and `std.io.stdout.color`
//! report the run's [`StreamColor`].

use std::ffi::OsString;
use std::io::IsTerminal;
use std::sync::OnceLock;

/// Whether each of the process's console streams may carry ANSI color,
/// reported to AXL as `std.io.<stream>.color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamColor {
    pub stdout: bool,
    pub stderr: bool,
}

impl StreamColor {
    /// The process's streams and environment, resolved on the first call.
    pub fn of_process() -> Self {
        static PROCESS: OnceLock<StreamColor> = OnceLock::new();
        *PROCESS.get_or_init(|| {
            Self::from_snapshot(
                std::io::stdout().is_terminal(),
                std::io::stderr().is_terminal(),
                |name| std::env::var_os(name),
            )
        })
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

/// Whether stdout may carry ANSI color: [`stdout_color_from`] over the
/// process ([`StreamColor::of_process`]).
pub fn stdout_supports_color() -> bool {
    StreamColor::of_process().stdout
}

/// Whether the lines the runtime prints to stderr (the console slots, `WARNING:`
/// and other severity prefixes, the grey Bazel trace line) may carry ANSI
/// color: [`console_color_from`] over the process ([`StreamColor::of_process`]).
/// AXL reads it as `std.io.stderr.color`.
pub fn console_supports_color() -> bool {
    StreamColor::of_process().stderr
}

/// The stdout rule over a snapshot (see the module docs). `var` reads one
/// environment variable.
fn stdout_color_from(stdout_tty: bool, var: impl Fn(&str) -> Option<OsString>) -> bool {
    color_from(stdout_tty, false, &var)
}

/// The stderr rule over a snapshot (see the module docs): the stdout rule,
/// plus a CI host whose log viewer renders ANSI. `var` reads one environment
/// variable.
fn console_color_from(stderr_tty: bool, var: impl Fn(&str) -> Option<OsString>) -> bool {
    let ci_renders = crate::ci::renders_ansi(|name| non_empty(var(name)));
    color_from(stderr_tty, ci_renders, &var)
}

/// The module's rules in order, for a stream that is a terminal (`tty`) or
/// whose reader renders ANSI anyway (`renders_ansi`).
fn color_from(tty: bool, renders_ansi: bool, var: &impl Fn(&str) -> Option<OsString>) -> bool {
    if non_empty(var("NO_COLOR")) {
        return false;
    }
    if non_empty(var("CLICOLOR_FORCE")) {
        return true;
    }
    let clicolor = var("CLICOLOR");
    if clicolor.as_deref().is_some_and(|v| v == "0") {
        return false;
    }
    let term_colors = var("TERM").is_some_and(|t| t != "dumb");
    let tty_colors = tty && (term_colors || clicolor.is_some() || var("CI").is_some());
    tty_colors || renders_ansi
}

fn non_empty(value: Option<OsString>) -> bool {
    value.is_some_and(|v| !v.is_empty())
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

    const TERM: (&str, &str) = ("TERM", "xterm-256color");

    #[test]
    fn stdout_color_only_on_a_color_terminal_without_no_color() {
        assert!(stdout_color_from(true, env(&[TERM])));
        assert!(!stdout_color_from(false, env(&[TERM])));
        assert!(!stdout_color_from(
            false,
            env(&[TERM, ("GITHUB_ACTIONS", "true")])
        ));
        assert!(!stdout_color_from(true, env(&[TERM, ("NO_COLOR", "1")])));
        assert!(stdout_color_from(true, env(&[TERM, ("NO_COLOR", "")])));
    }

    #[test]
    fn a_terminal_needs_a_color_term_clicolor_or_ci() {
        assert!(!stdout_color_from(true, env(&[])), "no TERM");
        assert!(!stdout_color_from(true, env(&[("TERM", "dumb")])));
        assert!(!console_color_from(true, env(&[("TERM", "dumb")])));
        assert!(stdout_color_from(
            true,
            env(&[("TERM", "dumb"), ("CLICOLOR", "1")])
        ));
        assert!(stdout_color_from(true, env(&[("CI", "")])));
    }

    #[test]
    fn clicolor_force_and_clicolor_zero() {
        assert!(stdout_color_from(false, env(&[("CLICOLOR_FORCE", "1")])));
        assert!(console_color_from(false, env(&[("CLICOLOR_FORCE", "1")])));
        assert!(!stdout_color_from(false, env(&[("CLICOLOR_FORCE", "")])));
        assert!(!stdout_color_from(
            false,
            env(&[("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")])
        ));
        assert!(stdout_color_from(
            true,
            env(&[TERM, ("CLICOLOR_FORCE", "1"), ("CLICOLOR", "0")])
        ));
        assert!(!stdout_color_from(true, env(&[TERM, ("CLICOLOR", "0")])));
        assert!(!console_color_from(
            false,
            env(&[("BUILDKITE", "true"), ("CLICOLOR", "0")])
        ));
    }

    #[test]
    fn console_color_on_a_tty_or_an_ansi_rendering_ci_host() {
        assert!(console_color_from(true, env(&[TERM])));
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
        let tty = StreamColor::from_snapshot(true, true, env(&[TERM, ("NO_COLOR", "1")]));
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
        assert!(!console_color_from(true, env(&[TERM, ("NO_COLOR", "1")])));
        assert!(!console_color_from(
            false,
            env(&[("GITHUB_ACTIONS", "true"), ("NO_COLOR", "0")])
        ));
        assert!(console_color_from(true, env(&[TERM, ("NO_COLOR", "")])));
        assert!(console_color_from(
            false,
            env(&[("GITLAB_CI", "true"), ("NO_COLOR", "")])
        ));
    }
}
