/// Shell-like tokenizer per Bazel spec §4.
pub(crate) fn tokenize(line: &str) -> Vec<String> {
    #[derive(PartialEq)]
    enum State {
        Normal,
        SingleQuoted,
        DoubleQuoted,
    }

    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut state = State::Normal;
    let mut chars = line.chars().peekable();
    // Whether current token has started (non-whitespace seen since last boundary)
    let mut in_token = false;

    while let Some(ch) = chars.next() {
        match state {
            State::Normal => match ch {
                '#' => {
                    // Comment: if at token boundary, stop line; if mid-token, stop entire line
                    break;
                }
                ' ' | '\t' => {
                    if in_token {
                        tokens.push(std::mem::take(&mut current));
                        in_token = false;
                    }
                }
                '\'' => {
                    in_token = true;
                    state = State::SingleQuoted;
                }
                '"' => {
                    in_token = true;
                    state = State::DoubleQuoted;
                }
                '\\' => {
                    in_token = true;
                    // Consume next char as literal; if EOL, dangling backslash is dropped
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                    // else: dangling \ at EOL, silently drop
                }
                _ => {
                    in_token = true;
                    current.push(ch);
                }
            },
            State::SingleQuoted => match ch {
                '\'' => {
                    state = State::Normal;
                }
                '\\' => {
                    // Backslash escape inside single quotes
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                _ => {
                    current.push(ch);
                }
            },
            State::DoubleQuoted => match ch {
                '"' => {
                    state = State::Normal;
                }
                '\\' => {
                    // Backslash escape inside double quotes
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                _ => {
                    current.push(ch);
                }
            },
        }
    }

    // Handle unterminated quote or trailing token
    if in_token || !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple() {
        assert_eq!(tokenize("build --jobs=4"), vec!["build", "--jobs=4"]);
    }

    #[test]
    fn test_comment_at_boundary() {
        assert_eq!(tokenize("build # comment"), vec!["build"]);
    }

    #[test]
    fn test_comment_mid_token() {
        // # mid-token stops entire line
        assert_eq!(tokenize("build --flag#val"), vec!["build", "--flag"]);
    }

    #[test]
    fn test_single_quotes() {
        assert_eq!(
            tokenize("build '--flag=hello world'"),
            vec!["build", "--flag=hello world"]
        );
    }

    #[test]
    fn test_double_quotes() {
        assert_eq!(
            tokenize(r#"build "--flag=hello world""#),
            vec!["build", "--flag=hello world"]
        );
    }

    #[test]
    fn test_backslash_escape() {
        assert_eq!(
            tokenize(r"build --flag=hello\ world"),
            vec!["build", "--flag=hello world"]
        );
    }

    #[test]
    fn test_unterminated_quote() {
        // Unterminated quote extends to EOL, not an error
        assert_eq!(
            tokenize("build \"unterminated"),
            vec!["build", "unterminated"]
        );
    }

    #[test]
    fn test_empty_line() {
        assert_eq!(tokenize(""), Vec::<String>::new());
    }

    #[test]
    fn test_only_comment() {
        assert_eq!(tokenize("# just a comment"), Vec::<String>::new());
    }

    #[test]
    fn test_dangling_backslash() {
        // Dangling backslash at EOL is silently dropped
        let result = tokenize("build --flag\\");
        assert_eq!(result, vec!["build", "--flag"]);
    }

    #[test]
    fn test_backslash_in_single_quotes() {
        assert_eq!(tokenize("build '--fl\\ag'"), vec!["build", "--flag"]);
    }

    #[test]
    fn test_tab_separated() {
        assert_eq!(tokenize("build\tfoo\tbar"), vec!["build", "foo", "bar"]);
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(tokenize(""), Vec::<String>::new());
    }
}
