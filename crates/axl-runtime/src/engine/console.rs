//! The Starlark side of [`crate::console`]: reading the `Console` feature's
//! args into a resolved surface, and `ctx.console`, through which AXL renders
//! the `phase` and `progress` lines.

use std::sync::Arc;

use allocative::Allocative;
use derive_more::Display;
use starlark::collections::SmallMap;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::values::dict::DictRef;
use starlark::values::{
    NoSerialize, ProvidesStaticType, StarlarkValue, Value, ValueLike, starlark_value,
};
use starlark::{starlark_module, starlark_simple_value};

use crate::console::{Console as Surface, DEFAULT_PRESET, Overrides, Slot, TaskIdentity};

use super::arguments::Arguments;
use super::template::json_to_minijinja;

/// Resolve the surface for `task` from the `Console` feature's merged `args`.
///
/// A disabled feature gives the `full` layout. Fails on a malformed
/// `templates` value and on everything [`Surface::resolve`] refuses.
pub fn from_feature_args(args: &Arguments<'_>, task: TaskIdentity) -> Result<Surface, String> {
    if !args
        .get("enabled")
        .and_then(|v| v.unpack_bool())
        .unwrap_or(true)
    {
        return Ok(Surface::builtin(task));
    }
    let preset = args.get("template_preset").and_then(|v| v.unpack_str());
    // `args.custom` reaches the runtime as `None` until a config sets it.
    let overrides = match args.get("templates") {
        Some(v) if !v.is_none() => overrides_from_value(v)?,
        _ => Overrides::default(),
    };
    Surface::resolve(preset.unwrap_or(DEFAULT_PRESET), &overrides, task)
}

/// Narrow a `templates` dict to [`Overrides`]: a string value is a slot's
/// template, a dict value a task kind's block of them.
fn overrides_from_value(value: Value<'_>) -> Result<Overrides, String> {
    let dict = DictRef::from_value(value)
        .ok_or_else(|| format!("Console templates must be a dict, got {}", value.get_type()))?;
    let mut overrides = Overrides::default();
    for (key, entry) in dict.iter() {
        let key = key.unpack_str().ok_or_else(|| {
            format!(
                "Console templates keys must be strings, got {}",
                key.get_type()
            )
        })?;
        if let Some(template) = entry.unpack_str() {
            overrides.set(key, template.to_owned())?;
        } else if Slot::from_name(key).is_some() {
            return Err(format!(
                "Console templates[{key:?}] must be a template string, got {}",
                entry.get_type()
            ));
        } else if let Some(block) = DictRef::from_value(entry) {
            for (slot, template) in block.iter() {
                let (Some(slot), Some(template)) = (slot.unpack_str(), template.unpack_str())
                else {
                    return Err(format!(
                        "Console templates[{key:?}] must map slot names to template strings"
                    ));
                };
                overrides.set_for_kind(key, slot, template.to_owned())?;
            }
        } else {
            return Err(format!(
                "Console templates[{key:?}] must be a template string or a dict of them, got {}",
                entry.get_type()
            ));
        }
    }
    Ok(overrides)
}

/// `ctx.console`: the surface resolved for this run.
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Console>")]
pub struct Console(#[allocative(skip)] pub Arc<Surface>);

#[starlark_value(type = "Console")]
impl<'v> StarlarkValue<'v> for Console {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("console_methods", console_methods);
        Some(RES.methods())
    }
}

starlark_simple_value!(Console);

#[starlark_module]
fn console_methods(registry: &mut MethodsBuilder) {
    /// The `Console` feature's selected preset (`full`, `plain`, `compact` or
    /// `silent`), for task output that should follow it.
    #[starlark(attribute)]
    fn template_preset<'v>(this: Value<'v>) -> anyhow::Result<&'static str> {
        Ok(downcast(this)?.0.preset())
    }

    /// Render one slot (`banner`, `header`, `phase`, `progress`, `verdict` or
    /// `section`) with the kwargs as its context. Returns `""` when the
    /// layout leaves the slot blank; print nothing then. On Buildkite the
    /// section slots (`header`, `phase`, `verdict`, `section`) never render
    /// blank.
    fn render<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] slot: &str,
        #[starlark(kwargs)] context: SmallMap<String, Value<'v>>,
    ) -> anyhow::Result<String> {
        let console = downcast(this)?;
        let slot = Slot::from_name(slot)
            .ok_or_else(|| anyhow::anyhow!("console.render: unknown slot {slot:?}"))?;
        let mut json = serde_json::Map::new();
        for (k, v) in context {
            json.insert(k, v.to_json_value()?);
        }
        let context = json_to_minijinja(&serde_json::Value::Object(json));
        Ok(console.0.render(slot, context))
    }
}

fn downcast(this: Value<'_>) -> anyhow::Result<&Console> {
    this.downcast_ref::<Console>()
        .ok_or_else(|| anyhow::anyhow!("receiver is not a Console"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> TaskIdentity {
        TaskIdentity {
            kind: "check".into(),
            name: "check".into(),
            path: "check".into(),
            ..Default::default()
        }
    }

    /// Resolve a surface whose `templates` arg is the snippet's `templates`
    /// binding, then render `slot` with `text = "x"`.
    fn render(code: &str, slot: Slot) -> Result<String, String> {
        crate::test::eval(code).with_value("templates", |templates| {
            let args = Arguments::new();
            args.insert("templates".into(), templates);
            let surface = from_feature_args(&args, task())?;
            Ok(surface.render(slot, minijinja::context! { text => "x" }))
        })
    }

    #[test]
    fn a_templates_dict_layers_flat_then_per_kind() {
        let code = r#"templates = {"progress": "flat {{ text }}", "check": {"progress": "check {{ text }}"}}"#;
        assert_eq!(render(code, Slot::Progress).unwrap(), "check x");
        let code = r#"templates = {"progress": "flat {{ text }}"}"#;
        assert_eq!(render(code, Slot::Progress).unwrap(), "flat x");
    }

    #[test]
    fn unset_templates_keep_the_preset() {
        assert_eq!(render("templates = None", Slot::Progress).unwrap(), "→ x");
    }

    #[test]
    fn a_malformed_templates_value_is_refused() {
        for (code, expected) in [
            ("templates = [1]", "templates must be a dict, got list"),
            ("templates = {1: \"x\"}", "keys must be strings, got int"),
            (
                r#"templates = {"progress": 1}"#,
                r#"templates["progress"] must be a template string, got int"#,
            ),
            (
                r#"templates = {"verdict": {"check": "x"}}"#,
                r#"templates["verdict"] must be a template string, got dict"#,
            ),
            (
                r#"templates = {"check": 1}"#,
                r#"templates["check"] must be a template string or a dict of them, got int"#,
            ),
            (
                r#"templates = {"check": {"progress": 1}}"#,
                r#"templates["check"] must map slot names to template strings"#,
            ),
            (
                r#"templates = {"check": {"verdcit": "x"}}"#,
                r#"templates["check"]: "verdcit" is not a slot"#,
            ),
        ] {
            let err = render(code, Slot::Progress).unwrap_err();
            assert!(err.contains(expected), "{code}: {err}");
        }
    }

    #[test]
    fn a_disabled_feature_or_unset_preset_gives_full() {
        crate::test::eval("enabled = False").with_value("enabled", |enabled| {
            let args = Arguments::new();
            args.insert("enabled".into(), enabled);
            let surface = from_feature_args(&args, task()).unwrap();
            assert_eq!(surface.preset(), "full");
        });
        let surface = from_feature_args(&Arguments::new(), task()).unwrap();
        assert_eq!(surface.preset(), DEFAULT_PRESET);
    }

    #[test]
    fn ctx_console_renders_from_axl() {
        let code = r#"
def _impl(ctx):
    if ctx.console.template_preset != "full":
        fail("preset: " + ctx.console.template_preset)
    line = ctx.console.render("progress", text = "x", title = "Build", emoji = "", highlight_style = "", highlight_reset = "")
    if line != "→ Build · x":
        fail("rendered: " + repr(line))
    return 0

check = task(implementation = _impl)
"#;
        assert_eq!(crate::test::eval(code).run_task(0).unwrap(), Some(0));
    }

    #[test]
    fn ctx_console_refuses_an_unknown_slot() {
        let code = r#"
def _impl(ctx):
    ctx.console.render("verdcit")
    return 0

check = task(implementation = _impl)
"#;
        let err = crate::test::eval(code).run_task(0).unwrap_err();
        assert!(
            format!("{err:?}").contains("unknown slot \"verdcit\""),
            "{err:?}"
        );
    }
}
