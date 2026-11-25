//! `ctx.theme` in a `config.axl`: the colour theme's settings for everyone
//! working in the repository (`.aspect/config.axl`) or for the user
//! (`~/.aspect/config.axl`), below the user's `~/.aspect/config.json` (see
//! [`crate::theme`]).
//!
//! ```python
//! def config(ctx: ConfigContext):
//!     ctx.theme.select(name = "colorblind", appearance = "auto", bazel = "shared")
//!     ctx.theme.roles({"accent": "#FF8A3D"})
//!     ctx.theme.roles({"error": 160}, appearance = "light")
//!     ctx.theme.define("team-dark", {"extends": "aspect-dark", "roles": {"accent": 208}})
//! ```
//!
//! Each call is recorded for the layer of the file that made it, a later call
//! of the same file winning, and the run's theme is resolved again once every
//! config has run. A bad `appearance` or `bazel` is a warning naming the file
//! and is left out; a bad role or theme is reported when the theme is
//! resolved, the same way. Nothing here fails a config.

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::dict::DictRef;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::{NoSerialize, ProvidesStaticType, StarlarkValue, Value, starlark_value};

use crate::engine::store::Env;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<ConfigTheme>")]
pub struct ConfigTheme;

starlark_simple_value!(ConfigTheme);

#[starlark_value(type = "ConfigTheme")]
impl<'v> StarlarkValue<'v> for ConfigTheme {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("config_theme_methods", config_theme_methods);
        Some(RES.methods())
    }
}

/// Run `f` on the layer of the `config.axl` calling, its source named as
/// warnings name it.
fn with_layer(
    eval: &Evaluator,
    call: &str,
    f: impl FnOnce(&mut aspect_theme::Layer, &str),
) -> anyhow::Result<()> {
    let env = Env::from_eval(eval)?;
    let path = Env::current_script_path(eval)?;
    let source = format!("{} {call}", path.display());
    let mut layers = env
        .theme_config
        .lock()
        .map_err(|_| anyhow::anyhow!("the theme's config is poisoned"))?;
    let user = env.theme_inputs.is_user_config(&path);
    let layer = if user {
        &mut layers.user
    } else {
        &mut layers.repo
    };
    if layer.source.is_empty() {
        layer.source = format!("{} ctx.theme", path.display());
    }
    f(layer, &source);
    Ok(())
}

fn object(value: Value, what: &str) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    if DictRef::from_value(value).is_none() {
        return Err(anyhow::anyhow!(
            "{what} must be a dict, got {}",
            value.get_type()
        ));
    }
    match value.to_json_value()? {
        serde_json::Value::Object(map) => Ok(map),
        _ => Err(anyhow::anyhow!("{what} must be a dict with string keys")),
    }
}

#[starlark_module]
fn config_theme_methods(registry: &mut MethodsBuilder) {
    /// Pick the theme: `name`, a family (`aspect`, `high-contrast`,
    /// `colorblind`, `ansi`, `mono`, or one defined with `define` or a
    /// theme file) or a variant (`aspect-light`); `appearance`, `auto`,
    /// `light` or `dark`; `bazel`, `shared` (Bazel's labels take the theme's
    /// colours) or `tool` (Bazel's own). Each is optional, and the user's
    /// `~/.aspect/config.json` overrides each.
    fn select<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] name: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] appearance: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] bazel: NoneOr<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        with_layer(eval, "ctx.theme.select", |layer, source| {
            if let Some(name) = name.into_option() {
                layer.theme = Some(serde_json::Value::String(name.to_string()));
            }
            let text = |v: &str| serde_json::Value::String(v.to_string());
            if let Some(a) = appearance.into_option() {
                match aspect_theme::Appearance::parse_setting(&text(a)) {
                    Ok(a) => layer.appearance = Some(a),
                    Err(e) => crate::diag::warn(&format!("theme: {source}: {e}; ignoring it")),
                }
            }
            if let Some(b) = bazel.into_option() {
                match aspect_theme::Bazel::parse(&text(b)) {
                    Ok(b) => layer.bazel = Some(b),
                    Err(e) => crate::diag::warn(&format!("theme: {source}: {e}; ignoring it")),
                }
            }
        })?;
        Ok(NoneType)
    }

    /// Override roles of whichever theme is selected: `roles` maps a role
    /// name to a value as a theme file writes it (`"#FF5555"`, `196`,
    /// `{"fg": "@accent", "bold": True}`, `{"rgb": …, "256": …, "16": …}`).
    /// With `appearance` (`light` or `dark`), only when the selected theme
    /// has that appearance. The user's own overrides apply on top.
    fn roles<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] roles: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] appearance: NoneOr<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let roles = object(roles, "roles")?;
        let appearance = appearance.into_option().map(str::to_string);
        if let Some(a) = appearance
            .as_deref()
            .filter(|a| *a != "light" && *a != "dark")
        {
            return Err(anyhow::anyhow!(
                "appearance is \"light\" or \"dark\", got {a:?}"
            ));
        }
        with_layer(eval, "ctx.theme.roles", |layer, _| match appearance {
            None => layer.roles.extend(roles),
            Some(a) => {
                let slot = layer
                    .roles
                    .entry(a)
                    .or_insert_with(|| serde_json::Value::Object(Default::default()));
                if let serde_json::Value::Object(map) = slot {
                    map.extend(roles);
                }
            }
        })?;
        Ok(NoneType)
    }

    /// Define a theme named `name`, as a theme file is written
    /// (`{"extends": "aspect-dark", "roles": {...}}`). Name two
    /// `<family>-dark` and `<family>-light` to make a family `select` can
    /// name. A name the user's `config.json` or theme files also define is
    /// theirs.
    fn define<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] theme: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let def = serde_json::Value::Object(object(theme, "a theme")?);
        with_layer(eval, "ctx.theme.define", |layer, _| {
            layer.themes.insert(name.to_string(), def);
        })?;
        Ok(NoneType)
    }
}

#[cfg(test)]
mod tests {
    const TASK: &str = r#"
def _impl(ctx):
    t = ctx.std.theme
    i = t.info()
    got = [i["name"], i["bazel"], t.params("accent"), t.params("error"), t.params("label.error"), t.params("tip.label")]
    if got != WANT:
        fail("got " + repr(got) + " want " + repr(WANT))
    return 0

t = task(implementation = _impl)
"#;

    fn run(want: &str, config: &str) -> anyhow::Result<Option<u8>> {
        crate::test::eval(&format!("WANT = {want}\n{TASK}"))
            .with_config(config)
            .with_host_env(false, false, &[("BUILDKITE", "true")])
            .run_task(0)
    }

    /// A repository's `config.axl` picks the theme, overrides roles for every
    /// appearance and for one, and defines a family of its own.
    #[test]
    fn a_config_axl_selects_overrides_and_defines() {
        let result = run(
            r#"["team-dark", "tool", "38;5;208", "38;5;196", "1;38;5;196", "1;38;5;208"]"#,
            r#"
def config(ctx):
    ctx.theme.define("team-dark", {"extends": "aspect-dark", "roles": {"accent": 208}})
    ctx.theme.define("team-light", {"extends": "aspect-light"})
    ctx.theme.select(name = "team", appearance = "auto", bazel = "tool")
    ctx.theme.roles({"error": 160})
    ctx.theme.roles({"error": 196}, appearance = "dark")
    ctx.theme.roles({"error": 88}, appearance = "light")
"#,
        );
        assert!(matches!(result, Ok(Some(0))), "{result:?}");
    }

    #[test]
    fn a_bad_setting_warns_and_a_bad_call_fails_only_that_call() {
        let result = run(
            r#"["aspect-dark", "shared", "38;5;75", "38;5;203", "1;38;5;203", "1;38;5;75"]"#,
            r#"
def config(ctx):
    ctx.theme.select(appearance = "dim", bazel = "off")
    ctx.theme.roles({"error": "purple"})
"#,
        );
        assert!(matches!(result, Ok(Some(0))), "{result:?}");
        let err = crate::test::eval(&format!("WANT = []\n{TASK}"))
            .with_config("def config(ctx):\n    ctx.theme.roles([\"error\"])\n")
            .run_task(0)
            .unwrap_err();
        assert!(
            format!("{err:?}").contains("roles must be a dict"),
            "{err:?}"
        );
    }
}
