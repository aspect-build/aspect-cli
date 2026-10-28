/// Join line continuations and strip whitespace.
pub(crate) fn process(content: &str) -> Vec<String> {
    // Join line continuations. Order matters: \\\r\n before \\\n.
    let joined = content.replace("\\\r\n", "").replace("\\\n", "");
    joined
        .split('\n')
        .map(|line| {
            line.trim_matches(|c| c == ' ' || c == '\t' || c == '\r')
                .to_owned()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_continuation() {
        let input = "build \\\n  --jobs=4\nbuild --verbose_failures";
        let result = process(input);
        assert_eq!(result, vec!["build   --jobs=4", "build --verbose_failures"]);
    }

    #[test]
    fn test_crlf_continuation() {
        let input = "build \\\r\n  --jobs=4";
        let result = process(input);
        assert_eq!(result, vec!["build   --jobs=4"]);
    }

    #[test]
    fn test_empty_lines_dropped() {
        let input = "build --jobs=4\n\n  \nbuild --verbose_failures";
        let result = process(input);
        assert_eq!(result, vec!["build --jobs=4", "build --verbose_failures"]);
    }

    #[test]
    fn test_whitespace_stripped() {
        let input = "  build --jobs=4  \t";
        let result = process(input);
        assert_eq!(result, vec!["build --jobs=4"]);
    }

    #[test]
    fn test_crlf_line_endings() {
        // CRLF without continuation: \r should be stripped from line ending
        let result = process("build --jobs=4\r\n");
        assert_eq!(result, vec!["build --jobs=4"]);
    }

    #[test]
    fn test_multiple_line_continuations() {
        // Chained \\ across 3 lines
        let input = "build \\\n  --jobs=4 \\\n  --verbose_failures";
        let result = process(input);
        assert_eq!(result, vec!["build   --jobs=4   --verbose_failures"]);
    }
}
