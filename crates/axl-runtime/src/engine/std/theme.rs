//! `ctx.std.theme`, also `load("@std//theme.axl", "theme")`: the run's
//! colour theme ([`crate::theme`]), a role at a time.
//!
//! The value holds nothing: every method reads the run's theme and stream
//! colour from the evaluator when it is called, so one loaded at a module's
//! top level answers for the run that calls it, background and all.
//!
//! `stream` is `"stderr"` (the default) or `"stdout"`, for that stream's
//! colour (`std.io.<stream>.color`), or a bool a caller has already decided:
//! `True` for a surface that draws regardless, such as a full-screen console
//! on the terminal (under `NO_COLOR` the theme is then `mono`). When neither
//! stream takes colour for another reason (both piped, `TERM=dumb`), the
//! run's theme writes no SGR, so even `True` gives `""`.
//!
//! The theme can change during a run: once its configs have run
//! (`ctx.theme`), and when the terminal reports its background
//! ([`crate::theme::learn_background`]). Each call answers for the theme as
//! it is then.

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::none::NoneOr;
use starlark::values::{NoSerialize, ProvidesStaticType, StarlarkValue, Value, starlark_value};

use crate::engine::store::Env as RuntimeEnv;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.Theme>")]
pub struct Theme;

starlark_simple_value!(Theme);

#[starlark_value(type = "std.Theme")]
impl<'v> StarlarkValue<'v> for Theme {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("theme_methods", theme_methods);
        Some(RES.methods())
    }
}

/// Whether `stream` takes colour in the run `env`.
fn gated(env: &RuntimeEnv, stream: Value) -> anyhow::Result<bool> {
    if let Some(on) = stream.unpack_bool() {
        return Ok(on);
    }
    match stream.unpack_str() {
        Some("stderr") => Ok(env.stream_color.stderr),
        Some("stdout") => Ok(env.stream_color.stdout),
        _ => Err(anyhow::anyhow!(
            "stream must be \"stderr\", \"stdout\", True or False, got {}",
            stream.to_repr()
        )),
    }
}

/// `role` of the run's theme, or `default` for a role it does not have.
fn entry(
    env: &RuntimeEnv,
    role: &str,
    default: NoneOr<&str>,
) -> anyhow::Result<aspect_theme::Entry> {
    let default = default
        .into_option()
        .map(|d| serde_json::Value::String(d.to_string()));
    env.theme
        .get()
        .entry_or(role, default.as_ref())
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// What a method that styles needs: the entry, when the stream takes colour.
fn shown(
    eval: &Evaluator,
    role: &str,
    stream: Value,
    default: NoneOr<&str>,
) -> anyhow::Result<Option<aspect_theme::Entry>> {
    let env = RuntimeEnv::from_eval(eval)?;
    let entry = entry(env, role, default)?;
    Ok(gated(env, stream)?.then_some(entry))
}

#[starlark_module]
fn theme_methods(registry: &mut MethodsBuilder) {
    /// The escape that opens `role` (`"\x1b[1;38;5;203m"`), or `""` when
    /// `stream` takes no colour or the role has no styling. `default` is a
    /// role value (`"@highlight"`) for a role of a plugin's own that the
    /// theme may not define. A role the theme does not have, with no
    /// `default`, fails.
    fn sgr<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] role: &str,
        #[starlark(require = named, default = "stderr")] stream: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<String> {
        Ok(shown(eval, role, stream, default)?
            .map(|e| e.sgr())
            .unwrap_or_default())
    }

    /// `text` in `role`, then a reset; `text` alone when `stream` takes no
    /// colour or the role has no styling. `default` as for `sgr`.
    fn paint<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] role: &str,
        #[starlark(require = pos)] text: &str,
        #[starlark(require = named, default = "stderr")] stream: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<String> {
        Ok(match shown(eval, role, stream, default)? {
            Some(e) if !e.params.is_empty() => {
                format!("{}{text}{}", e.sgr(), aspect_theme::RESET)
            }
            _ => text.to_string(),
        })
    }

    /// Only `role`'s foreground, as an escape (`"\x1b[38;5;203m"`); `""` when
    /// it has none or `stream` takes no colour. For a surface that combines
    /// one role's text colour with another's background.
    fn fg<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] role: &str,
        #[starlark(require = named, default = "stderr")] stream: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<String> {
        Ok(shown(eval, role, stream, default)?
            .map(|e| e.colour_sgr(false))
            .unwrap_or_default())
    }

    /// Only `role`'s background, as an escape (`"\x1b[48;5;24m"`), as `fg`
    /// is for the foreground: a selection, a hovered tab, a search match.
    fn bg<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] role: &str,
        #[starlark(require = named, default = "stderr")] stream: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<String> {
        Ok(shown(eval, role, stream, default)?
            .map(|e| e.colour_sgr(true))
            .unwrap_or_default())
    }

    /// `role`'s SGR parameters (`"1;38;5;203"`), whatever any stream takes:
    /// for composing a sequence of one's own. `""` for no styling.
    fn params<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] role: &str,
        #[starlark(require = named, default = NoneOr::None)] default: NoneOr<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<String> {
        Ok(entry(RuntimeEnv::from_eval(eval)?, role, default)?.params)
    }

    /// The colour depth `stream` is drawn at: 24 (24-bit), 256 or 16, and 0
    /// when it takes no colour.
    fn depth<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = named, default = "stderr")] stream: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<i32> {
        let env = RuntimeEnv::from_eval(eval)?;
        Ok(match (gated(env, stream)?, env.theme.get().depth()) {
            (true, Some(d)) => d.number() as i32,
            _ => 0,
        })
    }

    /// What the run's theme is: `name` (the variant, `aspect-dark`),
    /// `family` (`aspect`, or None), `appearance` (`dark`, `light` or
    /// `any`), `background` (`dark` or `light` when the terminal's is known,
    /// else None), `follows_terminal` (whether the background is unknown, so
    /// its status roles take the terminal's own colours), `bazel` (`shared`
    /// or `tool`), `hyperlinks` (whether the terminal opens OSC 8 links),
    /// `syntax_theme` (a syntect theme name, or None) and `roles` (every
    /// role name).
    fn info<'v>(
        #[allow(unused)] this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let palette = RuntimeEnv::from_eval(eval)?.theme.get();
        let theme = palette.theme();
        let info = serde_json::json!({
            "name": theme.name,
            "family": theme.family,
            "appearance": theme.appearance.name(),
            "background": theme.background.map(|a| a.name()),
            "follows_terminal": theme.follows_terminal,
            "bazel": match palette.bazel() {
                aspect_theme::Bazel::Shared => "shared",
                aspect_theme::Bazel::Tool => "tool",
            },
            "hyperlinks": palette.hyperlinks(),
            "syntax_theme": theme.syntax_theme,
            "roles": theme.roles().collect::<Vec<_>>(),
        });
        Ok(eval.heap().alloc(info))
    }

    /// Pin the run's theme to the built-in `name` (a family takes its
    /// `appearance` variant) at `depth` (24, 256 or 16), its background
    /// known: for a test that asserts the escapes a theme writes, whatever
    /// the environment it runs in. Only a test may call it: code in a
    /// `*_test.axl` file, or the runtime's own tests; anywhere else it fails.
    fn testonly_pin<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named, default = 256)] depth: i32,
        #[starlark(require = named, default = "dark")] appearance: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::none::NoneType> {
        let caller = RuntimeEnv::current_script_path(eval)?;
        let in_a_test = caller
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("_test.axl"));
        if !(cfg!(test) || in_a_test) {
            return Err(anyhow::anyhow!(
                "theme.testonly_pin is for tests: call it from a *_test.axl file, not {}",
                caller.display()
            ));
        }
        let depth = match depth {
            24 => aspect_theme::Depth::TrueColor,
            256 => aspect_theme::Depth::Ansi256,
            16 => aspect_theme::Depth::Ansi16,
            other => return Err(anyhow::anyhow!("depth is 24, 256 or 16, got {other}")),
        };
        let appearance = match appearance {
            "dark" => aspect_theme::Appearance::Dark,
            "light" => aspect_theme::Appearance::Light,
            other => {
                return Err(anyhow::anyhow!(
                    "appearance is dark or light, got {other:?}"
                ));
            }
        };
        let theme = aspect_theme::Library::default()
            .choose(name, Some(appearance), &mut || aspect_theme::Seen::Unknown)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let palette = aspect_theme::Palette::new(theme, Some(depth), false);
        RuntimeEnv::from_eval(eval)?
            .theme
            .set(std::sync::Arc::new(palette));
        Ok(starlark::values::none::NoneType)
    }

    /// The SGR reset, `"\x1b[0m"`.
    #[starlark(attribute)]
    fn reset<'v>(#[allow(unused)] this: Value<'v>) -> anyhow::Result<&'static str> {
        Ok(aspect_theme::RESET)
    }
}

#[cfg(test)]
mod tests {
    /// Run `body` (the statements of a task returning `got` and `want`) under
    /// the host snapshot; it fails with both when they differ.
    fn check(body: &str, stderr_tty: bool, vars: &[(&str, &str)]) {
        let code = format!(
            "{}\n    if got != want:\n        fail(\"got \" + repr(got) + \" want \" + repr(want))\n    return 0\n\ncheck = task(implementation = _impl)\n",
            body
        );
        let result = crate::test::eval(&code)
            .with_loader()
            .with_host_env(false, stderr_tty, vars)
            .run_task(0);
        assert!(matches!(result, Ok(Some(0))), "{result:?}");
    }

    const DARK_CI: &[(&str, &str)] = &[("BUILDKITE", "true")];

    #[test]
    fn a_role_for_a_stream_that_takes_colour() {
        check(
            r#"
def _impl(ctx):
    t = ctx.std.theme
    got = [
        t.sgr("label.error"),
        t.paint("muted", "x"),
        t.sgr("error", stream = "stdout"),
        t.sgr("error", stream = True),
        t.sgr("error", stream = False),
        t.fg("search.match"),
        t.bg("search.match"),
        t.bg("selection"),
        t.params("title"),
        t.depth(),
        t.depth(stream = "stdout"),
        t.reset,
    ]
    want = [
        "\x1b[1;38;5;203m",
        "\x1b[38;5;247mx\x1b[0m",
        "",
        "\x1b[38;5;203m",
        "",
        "\x1b[38;5;230m",
        "\x1b[48;5;58m",
        "\x1b[48;5;24m",
        "1;4;38;5;75",
        256,
        0,
        "\x1b[0m",
    ]"#,
            false,
            DARK_CI,
        );
    }

    #[test]
    fn no_color_writes_no_sgr_but_a_surface_that_draws_anyway_gets_mono() {
        check(
            r#"
def _impl(ctx):
    t = ctx.std.theme
    got = [t.sgr("error"), t.paint("error", "x"), t.depth(), t.sgr("active", stream = True), t.sgr("muted", stream = True), t.info()["name"]]
    want = ["", "x", 0, "\x1b[7m", "", "mono"]"#,
            true,
            &[("NO_COLOR", "1"), ("TERM", "xterm-256color")],
        );
    }

    #[test]
    fn a_plugin_role_takes_its_default_and_an_unknown_role_fails() {
        check(
            r#"
def _impl(ctx):
    got = ctx.std.theme.sgr("go.command", default = "@highlight")
    want = "\x1b[1;38;5;75m""#,
            false,
            DARK_CI,
        );
        let result = crate::test::eval(
            "def _impl(ctx):\n    return ctx.std.theme.sgr(\"eror\")\n\ncheck = task(implementation = _impl)\n",
        )
        .with_host_env(false, false, DARK_CI)
        .run_task(0);
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.contains("did you mean \\\"error\\\"?") || err.contains("did you mean \"error\"?"),
            "{err}"
        );
    }

    /// With the background unknown the status roles follow the terminal, and
    /// the module's top level reads the same run as the task does.
    #[test]
    fn info_and_the_std_module_at_top_level() {
        check(
            r#"
load("@std//theme.axl", "theme")

_ERROR = theme.sgr("label.error")

def _impl(ctx):
    i = ctx.std.theme.info()
    got = [_ERROR, theme.sgr("label.error"), theme.sgr("accent"), i["name"], i["family"], i["follows_terminal"], i["bazel"], i["hyperlinks"], "bazel.status.success" in i["roles"]]
    want = ["\x1b[1;31m", "\x1b[1;31m", "\x1b[38;5;75m", "aspect-dark", "aspect", True, "shared", False, True]"#,
            true,
            &[("TERM", "xterm-256color")],
        );
    }

    #[test]
    fn a_bad_stream_fails() {
        let result = crate::test::eval(
            "def _impl(ctx):\n    return ctx.std.theme.sgr(\"error\", stream = \"tty\")\n\ncheck = task(implementation = _impl)\n",
        )
        .with_host_env(false, false, DARK_CI)
        .run_task(0);
        let err = format!("{:?}", result.unwrap_err());
        assert!(err.contains("stream must be"), "{err}");
    }
}

#[cfg(test)]
mod info_tests {
    fn check(body: &str, stderr_tty: bool, vars: &[(&str, &str)]) {
        let code = format!(
            "{body}\n    if got != want:\n        fail(\"got \" + repr(got) + \" want \" + repr(want))\n    return 0\n\ncheck = task(implementation = _impl)\n"
        );
        let result = crate::test::eval(&code)
            .with_host_env(false, stderr_tty, vars)
            .run_task(0);
        assert!(matches!(result, Ok(Some(0))), "{result:?}");
    }

    #[test]
    fn info_says_whether_the_background_is_known() {
        let body = r#"
def _impl(ctx):
    got = ctx.std.theme.info()["background"]
    want = WANT"#;
        check(
            &body.replace("WANT", "\"dark\""),
            false,
            &[("BUILDKITE", "true")],
        );
        check(
            &body.replace("WANT", "\"light\""),
            true,
            &[("TERM", "xterm-256color"), ("COLORFGBG", "0;15")],
        );
        check(
            &body.replace("WANT", "None"),
            true,
            &[("TERM", "xterm-256color")],
        );
    }

    /// A pinned theme answers the same whatever the run's environment: here
    /// one whose streams take no colour.
    #[test]
    fn a_pinned_theme_ignores_the_environment() {
        check(
            r#"
def _impl(ctx):
    t = ctx.std.theme
    before = t.sgr("label.error", stream = True)
    t.testonly_pin("colorblind", depth = 16, appearance = "light")
    got = [before, t.info()["name"], t.sgr("label.error", stream = True), t.depth(stream = True)]
    want = ["", "colorblind-light", "\x1b[1;31m", 16]"#,
            false,
            &[],
        );
    }
}
