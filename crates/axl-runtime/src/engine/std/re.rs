//! `ctx.std.re`: regular expressions, in Rust's `regex` syntax, which runs in
//! time linear in its input whatever the pattern, so a pattern a user types
//! cannot hang the caller.
//!
//! A pattern is compiled once ([`Pattern`]) and then tested against text,
//! used to filter a list of lines, to replace or highlight its matches, each in one
//! call, so a feature redrawing thousands of lines a frame pays for one
//! crossing into Rust, not one per line. Starlark has no `try`, so a pattern
//! that may be invalid (one being typed) is checked first with
//! `std.re.check`, under the same options, which returns the error rather
//! than raising it.

use allocative::Allocative;
use derive_more::Display;
use starlark::StarlarkResultExt;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneOr;
use starlark::values::{self, NoSerialize, ProvidesStaticType, ValueLike, starlark_value};

/// `ctx.std.re`.
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.re.Re>")]
pub struct Re;

starlark_simple_value!(Re);

#[starlark_value(type = "std.re.Re")]
impl<'v> values::StarlarkValue<'v> for Re {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("re_methods", re_methods);
        Some(RES.methods())
    }
}

/// The regex `pattern` means with the options: case-insensitive unless
/// `case_sensitive`, and matching only whole words with `whole_word` (the
/// pattern between word boundaries).
fn compile_regex(
    pattern: &str,
    case_sensitive: bool,
    whole_word: bool,
) -> Result<regex::Regex, regex::Error> {
    let source = if whole_word {
        // Valid alone first: `a)|(b` would otherwise close the group and
        // escape the boundaries, wrapped into a pattern that compiles.
        regex::RegexBuilder::new(pattern)
            .case_insensitive(!case_sensitive)
            .build()?;
        format!(r"\b(?:{pattern})\b")
    } else {
        pattern.to_string()
    };
    regex::RegexBuilder::new(&source)
        .case_insensitive(!case_sensitive)
        .build()
}

/// A regex error's message without the pattern repeated above it: the last
/// line, `error: …`, which says what is wrong.
fn message(e: &regex::Error) -> String {
    let text = e.to_string();
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().trim_start_matches("error: ").to_string())
        .unwrap_or(text)
}

#[starlark_module]
fn re_methods(registry: &mut MethodsBuilder) {
    /// Compile `pattern` (Rust `regex` syntax). Case-insensitive unless
    /// `case_sensitive`; with `whole_word`, a match must start and end at a
    /// word boundary. Fails on an invalid pattern; see `check`.
    fn compile<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] pattern: &str,
        #[starlark(require = named, default = true)] case_sensitive: bool,
        #[starlark(require = named, default = false)] whole_word: bool,
    ) -> anyhow::Result<Pattern> {
        let regex = compile_regex(pattern, case_sensitive, whole_word)
            .map_err(|e| anyhow::anyhow!("invalid pattern {pattern:?}: {}", message(&e)))?;
        Ok(Pattern {
            regex,
            source: pattern.to_string(),
            case_sensitive,
            whole_word,
        })
    }

    /// What is wrong with `pattern` under the same options as `compile`,
    /// or None when `compile` would take it: for a pattern still being
    /// typed, which `compile` would fail on.
    fn check<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] pattern: &str,
        #[starlark(require = named, default = true)] case_sensitive: bool,
        #[starlark(require = named, default = false)] whole_word: bool,
    ) -> anyhow::Result<NoneOr<String>> {
        Ok(match compile_regex(pattern, case_sensitive, whole_word) {
            Ok(_) => NoneOr::None,
            Err(e) => NoneOr::Other(message(&e)),
        })
    }
}

/// A compiled pattern: `std.re.compile`.
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.re.Pattern {source:?}>")]
pub struct Pattern {
    #[allocative(skip)]
    regex: regex::Regex,
    source: String,
    case_sensitive: bool,
    whole_word: bool,
}

starlark_simple_value!(Pattern);

#[starlark_value(type = "std.re.Pattern")]
impl<'v> values::StarlarkValue<'v> for Pattern {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("pattern_methods", pattern_methods);
        Some(RES.methods())
    }
}

fn pattern_of<'v>(this: values::Value<'v>) -> anyhow::Result<&'v Pattern> {
    this.downcast_ref_err::<Pattern>().into_anyhow_result()
}

#[starlark_module]
fn pattern_methods(registry: &mut MethodsBuilder) {
    /// The pattern as written, before the options.
    #[starlark(attribute)]
    fn pattern<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(pattern_of(this)?.source.clone())
    }

    #[starlark(attribute)]
    fn case_sensitive<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        Ok(pattern_of(this)?.case_sensitive)
    }

    #[starlark(attribute)]
    fn whole_word<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        Ok(pattern_of(this)?.whole_word)
    }

    /// Whether the pattern matches anywhere in `text`.
    fn search<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] text: &str,
    ) -> anyhow::Result<bool> {
        Ok(pattern_of(this)?.regex.is_match(text))
    }

    /// The lines of `lines` the pattern matches, in order.
    fn filter<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] lines: UnpackList<String>,
    ) -> anyhow::Result<Vec<String>> {
        let regex = &pattern_of(this)?.regex;
        Ok(lines
            .items
            .into_iter()
            .filter(|l| regex.is_match(l))
            .collect())
    }

    /// `text` with every match replaced by `repl`, literally (no `$1`
    /// expansion): stripping escape sequences from a log, say.
    fn replace<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] text: &str,
        #[starlark(require = pos)] repl: &str,
    ) -> anyhow::Result<String> {
        Ok(pattern_of(this)?
            .regex
            .replace_all(text, regex::NoExpand(repl))
            .into_owned())
    }

    /// `text` with each non-empty match between `before` and `after` (an
    /// escape sequence that turns a highlight on and off, say).
    fn highlight<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] text: &str,
        #[starlark(require = pos)] before: &str,
        #[starlark(require = pos)] after: &str,
    ) -> anyhow::Result<String> {
        let regex = &pattern_of(this)?.regex;
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for m in regex.find_iter(text).filter(|m| !m.is_empty()) {
            out.push_str(&text[at..m.start()]);
            out.push_str(before);
            out.push_str(m.as_str());
            out.push_str(after);
            at = m.end();
        }
        out.push_str(&text[at..]);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_change_what_matches() {
        let ci = compile_regex("error", false, false).unwrap();
        assert!(ci.is_match("ERROR: x"));
        let cs = compile_regex("error", true, false).unwrap();
        assert!(!cs.is_match("ERROR: x"));
        let word = compile_regex("fail", false, true).unwrap();
        assert!(word.is_match("tests fail here"));
        assert!(!word.is_match("tests failed"));
        // An alternation stays inside the boundaries.
        assert!(compile_regex("a)|(b", true, true).is_err());
        let alt = compile_regex("a|b", true, true).unwrap();
        assert!(!alt.is_match("xa"));
    }

    /// Validity depends on the options: a comment in verbose mode swallows
    /// the word boundaries' closing group.
    #[test]
    fn validity_depends_on_the_options() {
        assert!(compile_regex("(?x)a # c", true, false).is_ok());
        assert!(compile_regex("(?x)a # c", true, true).is_err());
    }

    #[test]
    fn replace_is_literal() {
        let re = compile_regex("x+", true, false).unwrap();
        assert_eq!(re.replace_all("axxbx", regex::NoExpand("$1")), "a$1b$1");
    }

    #[test]
    fn an_invalid_pattern_says_what_is_wrong() {
        let e = compile_regex("(unclosed", true, false).unwrap_err();
        let m = message(&e);
        assert!(m.contains("unclosed"), "{m}");
        assert!(!m.contains('\n'), "{m}");
    }
}
