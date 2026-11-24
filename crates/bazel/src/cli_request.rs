//! Requests the CLI's own features put on a Bazel command line as flag values
//! Bazel must never see: they ride a task's flags, so the user's flag filters
//! and version gates apply to them as to any injected flag, and every spawn
//! that renders the `build` rc section takes them off with [`take`].

/// What the CLI's features asked of a command, taken off its flags.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Requests {
    /// The compact execution log, in the task invocation's history
    /// ([`crate::execlog::IN_HISTORY`]).
    pub execlog_in_history: bool,
    /// Every action reported, for the CLI's own readers
    /// ([`crate::bes::sink::downstream::CLI_REQUEST`]).
    pub all_actions: bool,
}

/// `flags` without the CLI's requests, and what they were.
pub fn take(flags: Vec<String>) -> (Vec<String>, Requests) {
    let (flags, execlog_in_history) = crate::execlog::take_history_request(flags);
    let (flags, all_actions) = crate::bes::sink::downstream::take_cli_request(flags);
    (
        flags,
        Requests {
            execlog_in_history,
            all_actions,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_requests_are_taken_off_and_nothing_else() {
        let flags: Vec<String> = [
            "--execution_log_compact_file=%aspect_history%",
            "--default_override=0:build=--build_event_publish_all_actions=%aspect_cli%",
            "--keep_going",
        ]
        .map(String::from)
        .to_vec();
        let (kept, asked) = take(flags);
        assert_eq!(kept, ["--keep_going"]);
        assert_eq!(
            asked,
            Requests {
                execlog_in_history: true,
                all_actions: true,
            }
        );
        assert_eq!(
            take(vec!["--a".into()]),
            (vec!["--a".into()], Requests::default())
        );
    }
}
