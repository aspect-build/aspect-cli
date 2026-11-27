//! `std.syntax`: syntax highlighting for a full-screen feature that shows a
//! source file (a peek at the line a log names), by Sublime Text's grammars
//! and themes ([`syntect`], pure-Rust regexes). The grammars load once, on
//! first use.

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::list::UnpackList;
use starlark::values::{self, NoSerialize, ProvidesStaticType, starlark_value};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

#[derive(Clone, Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.Syntax>")]
pub struct Syntax;

starlark_simple_value!(Syntax);

#[starlark_value(type = "std.Syntax")]
impl<'v> values::StarlarkValue<'v> for Syntax {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("syntax_methods", syntax_methods);
        Some(RES.methods())
    }
}

/// The grammars and the theme, loaded once.
static LOADED: std::sync::LazyLock<(SyntaxSet, Theme)> = std::sync::LazyLock::new(|| {
    let mut themes = ThemeSet::load_defaults().themes;
    let theme = themes
        .remove("base16-ocean.dark")
        .unwrap_or_else(|| themes.into_values().next().unwrap_or_default());
    (SyntaxSet::load_defaults_newlines(), theme)
});

#[starlark_module]
fn syntax_methods(registry: &mut MethodsBuilder) {
    /// `lines` of the file at `path` coloured by the grammar its name picks
    /// (`.rs`, `.py`, ...; a Starlark file's, `BUILD.bazel`, `MODULE.bazel`,
    /// `.bzl`, `.axl`, `.star` and the like, as Python), as escape sequences: in
    /// 24-bit colour with `truecolor`, else the nearest of the 256 colours.
    /// Lines a grammar does not know, or that are not text, come back as
    /// they are. Highlighting reads the lines in order, so passing a few
    /// before the ones shown starts a comment or a string where the file
    /// does.
    fn highlight<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] lines: UnpackList<String>,
        #[starlark(require = pos)] path: &str,
        #[starlark(require = named, default = false)] truecolor: bool,
    ) -> anyhow::Result<Vec<String>> {
        Ok(highlight_lines(&lines.items, path, truecolor))
    }
}

/// The file names of Starlark files, which the grammars, having none for
/// Starlark, colour as Python, a near enough superset.
const STARLARK_NAMES: &[&str] = &[
    "BUILD",
    "BUILD.bazel",
    "WORKSPACE",
    "WORKSPACE.bazel",
    "MODULE.bazel",
    "REPO.bazel",
];

/// The extensions of Starlark files ([`STARLARK_NAMES`]): Bazel's `.bzl`,
/// AXL's `.axl`, and Starlark's own `.star`.
const STARLARK_EXTENSIONS: &[&str] = &["bzl", "axl", "star"];

/// [`highlight`](syntax_methods)'s lines.
pub(crate) fn highlight_lines(lines: &[String], path: &str, truecolor: bool) -> Vec<String> {
    let (syntaxes, theme) = &*LOADED;
    let path = std::path::Path::new(path);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let extension = path.extension().map(|e| e.to_string_lossy().into_owned());
    let starlark = name.as_deref().is_some_and(|n| STARLARK_NAMES.contains(&n))
        || extension
            .as_deref()
            .is_some_and(|e| STARLARK_EXTENSIONS.contains(&e));
    let syntax = if starlark {
        syntaxes.find_syntax_by_extension("py")
    } else {
        extension
            .as_deref()
            .and_then(|e| syntaxes.find_syntax_by_extension(e))
    };
    let Some(syntax) = syntax else {
        return lines.to_vec();
    };
    let mut highlighter = HighlightLines::new(syntax, theme);
    lines
        .iter()
        .map(|line| {
            let with_newline = format!("{line}\n");
            match highlighter.highlight_line(&with_newline, syntaxes) {
                Ok(spans) => escaped(&spans, truecolor),
                Err(_) => line.clone(),
            }
        })
        .collect()
}

/// `spans` as foreground colours, each reset at the line's end, the newline
/// left out.
fn escaped(spans: &[(Style, &str)], truecolor: bool) -> String {
    let mut out = String::new();
    for (style, text) in spans {
        let text = text.trim_end_matches('\n');
        if text.is_empty() {
            continue;
        }
        let c = style.foreground;
        if truecolor {
            out.push_str(&format!("\x1b[38;2;{};{};{}m", c.r, c.g, c.b));
        } else {
            out.push_str(&format!("\x1b[38;5;{}m", nearest_256(c.r, c.g, c.b)));
        }
        out.push_str(text);
    }
    out.push_str("\x1b[39m");
    out
}

/// The nearest of the 256-colour palette's 6x6x6 cube and grey ramp.
fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let level = |v: u8| {
        (0..6)
            .min_by_key(|&i| (LEVELS[i] as i32 - v as i32).abs())
            .unwrap_or(0) as u8
    };
    let (ri, gi, bi) = (level(r), level(g), level(b));
    let cube = 16 + 36 * ri + 6 * gi + bi;
    let dist = |a: (u8, u8, u8)| {
        let d = |x: u8, y: u8| (x as i32 - y as i32).pow(2);
        d(a.0, r) + d(a.1, g) + d(a.2, b)
    };
    let cube_rgb = (
        LEVELS[ri as usize],
        LEVELS[gi as usize],
        LEVELS[bi as usize],
    );
    let grey_i = ((r as u32 + g as u32 + b as u32) / 3).saturating_sub(8) / 10;
    let grey_i = grey_i.min(23) as u8;
    let grey = 8 + 10 * grey_i;
    if dist((grey, grey, grey)) < dist(cube_rgb) {
        232 + grey_i
    } else {
        cube
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_language_is_coloured_and_an_unknown_one_is_not() {
        let rust = highlight_lines(&["fn main() {}".into()], "src/main.rs", false);
        assert!(rust[0].contains("\x1b[38;5;"), "{rust:?}");
        assert!(rust[0].ends_with("\x1b[39m"));
        let build = highlight_lines(&["load(\"//x\", \"y\")".into()], "pkg/BUILD.bazel", true);
        assert!(build[0].contains("\x1b[38;2;"), "{build:?}");
        let plain = highlight_lines(&["whatever".into()], "notes.unknownext", false);
        assert_eq!(plain, vec!["whatever".to_string()]);
    }

    /// Every Starlark file is coloured, by its name or its extension.
    #[test]
    fn starlark_files_are_coloured_as_python() {
        let line = vec!["def f(x):".to_string()];
        let python = highlight_lines(&line, "a.py", false);
        for path in [
            "BUILD",
            "pkg/BUILD.bazel",
            "WORKSPACE",
            "WORKSPACE.bazel",
            "MODULE.bazel",
            "REPO.bazel",
            "defs.bzl",
            ".aspect/config.axl",
            "x.star",
        ] {
            assert_eq!(highlight_lines(&line, path, false), python, "{path}");
        }
    }

    #[test]
    fn the_nearest_256_colour() {
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(255, 255, 255), 231);
        assert_eq!(nearest_256(128, 128, 128), 244);
    }
}
