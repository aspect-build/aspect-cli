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
/// `templates`, `task_presets` or `task_templates` value and on everything
/// [`Surface::resolve`] refuses.
pub fn from_feature_args(args: &Arguments<'_>, task: TaskIdentity) -> Result<Surface, String> {
    if !args
        .get("enabled")
        .and_then(|v| v.unpack_bool())
        .unwrap_or(true)
    {
        return Ok(Surface::builtin(task));
    }
    let preset = args.get("template_preset").and_then(|v| v.unpack_str());
    let mut overrides = Overrides::default();
    if args.is_explicit_key("template_preset") {
        overrides.preset_from_cli();
    }
    let templates = string_dict(args, "templates")?;
    blocks_error(&templates)?;
    for (slot, template) in templates {
        overrides.set(
            slot,
            string_value(template, || format!("templates[{slot:?}]"))?,
        )?;
    }
    for (task_key, name) in string_dict(args, "task_presets")? {
        overrides.set_preset_for_task(
            task_key,
            string_value(name, || format!("task_presets[{task_key:?}]"))?,
        )?;
    }
    for (task_key, block) in string_dict(args, "task_templates")? {
        let label = format!("task_templates[{task_key:?}]");
        let block = DictRef::from_value(block).ok_or_else(|| {
            format!(
                "Console {label} must be a dict of slot → template, got {}",
                block.get_type()
            )
        })?;
        for (slot, template) in block.iter() {
            let slot = slot.unpack_str().ok_or_else(|| {
                format!(
                    "Console {label} keys must be slot names, got {}",
                    slot.get_type()
                )
            })?;
            let template = string_value(template, || format!("{label}[{slot:?}]"))?;
            overrides.set_for_task(task_key, slot, template)?;
        }
    }
    Surface::resolve(preset.unwrap_or(DEFAULT_PRESET), &overrides, task)
}

/// Refuse the blocks in `templates` (dict values keyed by a task rather than a
/// slot) with one error spelling out the `task_templates` that replaces them.
/// Values are elided, and entries sorted by key, as on the CI surfaces.
fn blocks_error(templates: &[(&str, Value<'_>)]) -> Result<(), String> {
    let is_block = |(key, value): &&(&str, Value<'_>)| {
        Slot::from_name(key).is_none() && DictRef::from_value(*value).is_some()
    };
    let mut blocks: Vec<_> = templates.iter().filter(is_block).collect();
    blocks.sort_by_key(|(key, _)| *key);
    if blocks.is_empty() {
        return Ok(());
    }
    let elided = |keys: Vec<String>| {
        keys.iter()
            .map(|k| format!("{k:?}: ..."))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let slots = Slot::ALL.map(Slot::name).join(", ");
    let mut lines = vec![format!(
        "Console templates takes slot keys only ({slots}); a block keyed by a task goes in \
         `task_templates`, keyed by task. For this config:"
    )];
    let mut flat: Vec<String> = templates
        .iter()
        .filter(|entry| !is_block(entry))
        .map(|(key, _)| (*key).to_owned())
        .collect();
    flat.sort();
    if !flat.is_empty() {
        lines.push(format!("    templates = {{{}}}", elided(flat)));
    }
    lines.push("    task_templates = {".to_owned());
    for (task, block) in blocks {
        let block = DictRef::from_value(*block).expect("a block is a dict");
        let mut slots: Vec<String> = block
            .keys()
            .map(|slot| {
                slot.unpack_str()
                    .map_or_else(|| slot.to_repr(), str::to_owned)
            })
            .collect();
        slots.sort();
        lines.push(format!("        {task:?}: {{{}}},", elided(slots)));
    }
    lines.push("    }".to_owned());
    Err(lines.join("\n"))
}

/// The string-keyed entries of the dict arg `name`; none while it is unset,
/// since `args.custom` reaches the runtime as `None` until a config sets it.
fn string_dict<'v>(args: &Arguments<'v>, name: &str) -> Result<Vec<(&'v str, Value<'v>)>, String> {
    let Some(value) = args.get(name).filter(|v| !v.is_none()) else {
        return Ok(Vec::new());
    };
    let dict = DictRef::from_value(value)
        .ok_or_else(|| format!("Console {name} must be a dict, got {}", value.get_type()))?;
    dict.iter()
        .map(|(key, entry)| {
            let key = key.unpack_str().ok_or_else(|| {
                format!(
                    "Console {name} keys must be strings, got {}",
                    key.get_type()
                )
            })?;
            Ok((key, entry))
        })
        .collect()
}

/// `value` as a string, or an error naming it by `label`.
fn string_value(value: Value<'_>, label: impl FnOnce() -> String) -> Result<String, String> {
    value.unpack_str().map(str::to_owned).ok_or_else(|| {
        format!(
            "Console {} must be a string, got {}",
            label(),
            value.get_type()
        )
    })
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
    /// The preset this task renders with (`full`, `plain`, `compact` or
    /// `silent`): its `task_presets` entry, else the `Console` feature's
    /// `template_preset`, which wins outright when typed on the command line.
    /// For task output that should follow it.
    #[starlark(attribute)]
    fn template_preset<'v>(this: Value<'v>) -> anyhow::Result<&'static str> {
        Ok(downcast(this)?.0.preset())
    }

    /// Whether the console lines carry ANSI color: on an interactive stderr, or
    /// on GitHub Actions, Buildkite, CircleCI or GitLab CI, whose logs render
    /// it; never under a non-empty `NO_COLOR`. The runtime's own lines follow
    /// it; AXL that
    /// renders a slot gates the `highlight_style` it passes on it, as
    /// `lib/lifecycle.axl` does.
    #[starlark(attribute)]
    fn color<'v>(this: Value<'v>) -> anyhow::Result<bool> {
        Ok(downcast(this)?.0.color())
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

    /// Resolve a surface from the snippet's `templates`, `task_presets` and
    /// `task_templates` bindings (each `None` when unset), then render `slot`
    /// with `text = "x"`.
    fn render(code: &str, slot: Slot) -> Result<String, String> {
        let code = format!(
            r#"templates = None
task_presets = None
task_templates = None
{code}
args = {{"templates": templates, "task_presets": task_presets, "task_templates": task_templates}}"#
        );
        crate::test::eval(&code).with_value("args", |bound| {
            let args = Arguments::new();
            for (name, value) in DictRef::from_value(bound).unwrap().iter() {
                args.insert(name.unpack_str().unwrap().into(), value);
            }
            let surface = from_feature_args(&args, task())?;
            Ok(surface.render(slot, minijinja::context! { text => "x" }))
        })
    }

    #[test]
    fn templates_then_task_templates() {
        let code = r#"templates = {"progress": "flat {{ text }}"}
task_templates = {"check": {"progress": "check {{ text }}"}}"#;
        assert_eq!(render(code, Slot::Progress).unwrap(), "check x");
        let code = r#"templates = {"progress": "flat {{ text }}"}"#;
        assert_eq!(render(code, Slot::Progress).unwrap(), "flat x");
    }

    #[test]
    fn a_task_preset_leaves_out_templates() {
        let code = r#"templates = {"progress": "flat"}
task_presets = {"check": "silent"}"#;
        assert_eq!(render(code, Slot::Progress).unwrap(), "");
        let code = r#"task_presets = {"check": "silent"}
task_templates = {"check": {"progress": "mine"}}"#;
        assert_eq!(render(code, Slot::Progress).unwrap(), "mine");
    }

    #[test]
    fn unset_templates_keep_the_preset() {
        assert_eq!(render("", Slot::Progress).unwrap(), "→ x");
    }

    #[test]
    fn a_malformed_value_is_refused() {
        for (code, expected) in [
            ("templates = [1]", "templates must be a dict, got list"),
            (
                "task_presets = [1]",
                "task_presets must be a dict, got list",
            ),
            (
                "templates = {1: \"x\"}",
                "templates keys must be strings, got int",
            ),
            (
                r#"templates = {"progress": 1}"#,
                r#"templates["progress"] must be a string, got int"#,
            ),
            (
                r#"templates = {"lint": {"verdict": "v", "header": "h"}, "verdict": "V", "check": {"progress": "x"}}"#,
                "Console templates takes slot keys only (banner, header, phase, progress, verdict, section); \
                 a block keyed by a task goes in `task_templates`, keyed by task. For this config:\n    \
                 templates = {\"verdict\": ...}\n    task_templates = {\n        \
                 \"check\": {\"progress\": ...},\n        \"lint\": {\"header\": ..., \"verdict\": ...},\n    }",
            ),
            (
                r#"templates = {"verdict": {"x": "y"}}"#,
                r#"templates["verdict"] must be a string, got dict"#,
            ),
            (
                r#"task_presets = {"verdict": "silent"}"#,
                r#"task_presets["verdict"] names a slot"#,
            ),
            (
                r#"templates = {"verdcit": "x"}"#,
                r#"templates: "verdcit" is not a slot"#,
            ),
            (
                r#"task_presets = {"check": "loud"}"#,
                r#"task_presets["check"]: unknown template preset "loud""#,
            ),
            (
                r#"task_presets = {"check": 1}"#,
                r#"task_presets["check"] must be a string, got int"#,
            ),
            (
                r#"task_templates = {"check": 1}"#,
                r#"task_templates["check"] must be a dict of slot → template, got int"#,
            ),
            (
                r#"task_templates = {"check": {"progress": 1}}"#,
                r#"task_templates["check"]["progress"] must be a string, got int"#,
            ),
            (
                r#"task_templates = {"check": {"verdcit": "x"}}"#,
                r#"task_templates["check"]: "verdcit" is not a slot"#,
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
    return 1 if ctx.console.color else 0

check = task(implementation = _impl)
"#;
        let color = crate::color::console_supports_color();
        assert_eq!(
            crate::test::eval(code).run_task(0).unwrap(),
            Some(u8::from(color)),
            "ctx.console.color agrees with the runtime's own lines"
        );
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
