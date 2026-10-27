//! What to tell the user about how a build event stream ended.

use super::binary_file::End;

/// What to say about a build event stream that did not end with bazel's final
/// event, or `None` when there is nothing to say.
///
/// Bazel writes that event whenever it ends on its own terms, so its absence
/// means the writer died, was cut off, or never wrote. That is worth a warning
/// because the consumers of the stream (BES backends, hooks) saw less than the
/// build did. It is not worth one when bazel itself failed before writing a
/// single event (a rejected command line, say): its exit code already says so.
pub fn incomplete_events(end: Option<&End>, bazel_succeeded: bool) -> Option<String> {
    match end? {
        End::Complete => None,
        End::Incomplete(done) if !bazel_succeeded && done.bytes() == 0 => None,
        end => Some(format!(
            "WARNING: the build events were incomplete ({end}); anything consuming them \
             (BES backends, hooks) may have missed events."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::incomplete_events as incomplete_events_warning;
    use super::*;
    use crate::stream::Done;
    use std::time::Duration;

    fn ended(bytes: u64) -> End {
        End::Incomplete(Done::Ended {
            bytes,
            quiet: Duration::from_millis(5),
        })
    }

    #[test]
    fn a_stream_that_has_not_ended_has_nothing_to_say() {
        assert_eq!(incomplete_events_warning(None, true), None);
    }

    #[test]
    fn a_complete_stream_is_silent_whatever_the_exit_code() {
        assert_eq!(incomplete_events_warning(Some(&End::Complete), true), None);
        assert_eq!(incomplete_events_warning(Some(&End::Complete), false), None);
    }

    #[test]
    fn an_incomplete_stream_after_a_successful_build_warns() {
        let end = ended(10);
        let warning = incomplete_events_warning(Some(&end), true).unwrap();
        assert!(
            warning.starts_with("WARNING: the build events were incomplete"),
            "{warning}"
        );
        assert!(warning.contains("after 10 bytes"), "{warning}");
        // Even one that wrote nothing: a successful build must have written.
        assert!(incomplete_events_warning(Some(&ended(0)), true).is_some());
    }

    #[test]
    fn an_incomplete_stream_after_a_failed_build_warns_only_if_something_was_written() {
        assert!(incomplete_events_warning(Some(&ended(10)), false).is_some());
        assert_eq!(
            incomplete_events_warning(Some(&ended(0)), false),
            None,
            "bazel failed before writing anything; its exit code says so"
        );
        let empty_drain = End::Incomplete(Done::Drained { bytes: 0 });
        assert_eq!(incomplete_events_warning(Some(&empty_drain), false), None);
    }

    #[test]
    fn a_broken_stream_with_no_bytes_after_a_failed_build_is_silent_too() {
        let broken = End::Incomplete(Done::Broken {
            bytes: 0,
            error: std::io::Error::other("gone"),
        });
        assert_eq!(incomplete_events_warning(Some(&broken), false), None);
        assert!(incomplete_events_warning(Some(&broken), true).is_some());
    }

    #[test]
    fn a_corrupt_stream_always_warns() {
        let corrupt = End::Corrupt("bad varint".into());
        for succeeded in [true, false] {
            let warning = incomplete_events_warning(Some(&corrupt), succeeded).unwrap();
            assert!(warning.contains("corrupt: bad varint"), "{warning}");
        }
    }
}
