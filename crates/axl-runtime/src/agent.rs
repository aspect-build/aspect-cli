//! Whether a coding agent is running this process, from its environment alone.
//!
//! A run reads it once into its `Env` (`coding_agent`), and the `Console`
//! feature uses it to pick the `agent` layout when nothing else picks one.
//! `lib/agent.axl` answers the fuller question (which harness, which session)
//! with more layers, including a process-tree walk that is too slow for every
//! start-up; this is only its environment layers, and choosing a default
//! layout needs no more than one signal. [`ENV_VARS`] lists the same
//! variables `lib/agent.axl` reads, and a test in the `aspect-cli` crate keeps
//! the two in step.

use std::ffi::OsString;

/// Every variable whose non-empty value means a coding agent: the
/// vendor-neutral `ASPECT_AGENT_*` contract, the variables the known harnesses
/// set, and the generic `AI_AGENT`.
pub const ENV_VARS: &[&str] = &[
    "ASPECT_AGENT_KIND",
    "ASPECT_AGENT_ID",
    "CLAUDECODE",
    "CLAUDE_CODE_SESSION_ID",
    "CODEX_THREAD_ID",
    "GEMINI_CLI",
    "GEMINI_SESSION_ID",
    "GROK_SESSION_ID",
    "CURSOR_AGENT",
    "AI_AGENT",
];

/// Whether any of [`ENV_VARS`] is set to a non-empty value, read through `var`.
pub fn detected(var: impl Fn(&str) -> Option<OsString>) -> bool {
    ENV_VARS
        .iter()
        .any(|name| var(name).is_some_and(|v| !v.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_non_empty_signal_is_enough() {
        assert!(!detected(|_| None));
        assert!(!detected(|_| Some(OsString::new())));
        for signal in ENV_VARS {
            assert!(
                detected(|name| (name == *signal).then(|| "1".into())),
                "{signal}"
            );
        }
    }
}
