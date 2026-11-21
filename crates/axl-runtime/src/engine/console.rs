//! The Starlark side of [`crate::console`]: reading the `Console` feature's
//! args into a resolved surface for a given [`Host`], and `ctx.console`,
//! through which AXL renders the `phase` and `progress` lines.

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

use crate::console::{
    AGENT_PRESET, Console as Surface, DEFAULT_PRESET, Host, Overrides, Slot, TaskIdentity,
};

use super::arguments::Arguments;
use super::template::json_to_minijinja;

/// Resolve the surface for `task` on `host` from the `Console` feature's
/// merged `args`.
///
/// `template_preset` counts only when the command line or a `config.axl` set
/// it; otherwise the base preset is `agent` when [`Host::coding_agent`] and
/// `full` when not. A disabled feature gives the `full` layout. Fails on a
/// malformed `templates`, `task_presets`, `task_templates` or `info_lines`
/// value and on everything [`Surface::resolve`] refuses. The surface carries a
/// warning for each `task_presets` / `task_templates` key that is a near miss
/// of one of `task_paths`, the command paths of the repo's tasks.
pub fn from_feature_args(
    args: &Arguments<'_>,
    task: TaskIdentity,
    host: Host,
    task_paths: &[String],
) -> Result<Surface, String> {
    if !args
        .get("enabled")
        .and_then(|v| v.unpack_bool())
        .unwrap_or(true)
    {
        return Ok(Surface::builtin(task, host));
    }
    let chosen =
        args.is_explicit_key("template_preset") || args.is_configured_key("template_preset");
    let preset = match args.get("template_preset").and_then(|v| v.unpack_str()) {
        Some(name) if chosen => name,
        _ if host.coding_agent => AGENT_PRESET,
        _ => DEFAULT_PRESET,
    };
    let mut overrides = Overrides::default();
    if args.is_explicit_key("template_preset") {
        overrides.preset_from_cli();
    }
    if let Some(show) = info_lines(args)? {
        overrides.set_info_lines(show);
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
    Ok(Surface::resolve(preset, &overrides, task, host)?
        .with_warnings(overrides.near_miss_warnings(task_paths)))
}

/// The `info_lines` arg: `None` for `auto` or unset, so the preset decides.
/// The CLI gives a string; a `config.axl` may assign a bool too.
fn info_lines(args: &Arguments<'_>) -> Result<Option<bool>, String> {
    let Some(value) = args.get("info_lines").filter(|v| !v.is_none()) else {
        return Ok(None);
    };
    if let Some(show) = value.unpack_bool() {
        return Ok(Some(show));
    }
    match value.unpack_str() {
        Some("auto") => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        _ => Err(format!(
            "Console info_lines must be \"auto\", \"true\", \"false\" or a bool, got {}",
            value.to_repr()
        )),
    }
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
    /// The preset this task renders with (`full`, `plain`, `compact`,
    /// `agent` or `silent`): its `task_presets` entry, else the `Console`
    /// feature's `template_preset`, which wins outright when typed on the
    /// command line. With neither set, `agent` under a coding agent and `full`
    /// otherwise. For task output that should follow it.
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
        let slot = Slot::from_name(slot).ok_or_else(|| {
            anyhow::anyhow!(
                "console.render: unknown slot {slot:?}{}",
                crate::suggest::did_you_mean(slot, Slot::ALL.map(Slot::name))
            )
        })?;
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
            let surface = from_feature_args(&args, task(), Host::default(), &[])?;
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
            let surface = from_feature_args(&args, task(), Host::default(), &[]).unwrap();
            assert_eq!(surface.preset(), "full");
        });
        let surface = from_feature_args(&Arguments::new(), task(), Host::default(), &[]).unwrap();
        assert_eq!(surface.preset(), DEFAULT_PRESET);
    }

    /// Resolve with `template_preset` bound to `preset`, marked as `source`
    /// set it (`"cli"`, `"config"`, or `""` for the schema default), plus
    /// `extra` bindings.
    fn resolve_with(
        preset: &str,
        source: &str,
        extra: &str,
        coding_agent: bool,
    ) -> Result<Surface, String> {
        let code = format!(
            "info_lines = None\n{extra}\nargs = {{\"template_preset\": {preset:?}, \"info_lines\": info_lines}}"
        );
        crate::test::eval(&code).with_value("args", |bound| {
            let args = Arguments::new();
            for (name, value) in DictRef::from_value(bound).unwrap().iter() {
                args.insert(name.unpack_str().unwrap().into(), value);
            }
            match source {
                "cli" => args.mark_explicit("template_preset".into()),
                "config" => args.mark_configured("template_preset".into()),
                _ => {}
            }
            let host = Host {
                coding_agent,
                ..Host::default()
            };
            from_feature_args(&args, task(), host, &[])
        })
    }

    #[test]
    fn a_coding_agent_gets_agent_unless_a_preset_was_chosen() {
        let preset = |name, source, agent| resolve_with(name, source, "", agent).unwrap().preset();
        assert_eq!(preset("full", "", true), "agent");
        assert_eq!(preset("full", "", false), "full");
        assert_eq!(
            preset("full", "config", true),
            "full",
            "config beats detection"
        );
        assert_eq!(
            preset("plain", "cli", true),
            "plain",
            "the CLI beats detection"
        );
        assert_eq!(preset("compact", "config", false), "compact");
    }

    #[test]
    fn info_lines_overrides_the_preset() {
        let shown = |extra, preset| {
            resolve_with(preset, "cli", extra, false)
                .unwrap()
                .info_lines()
        };
        assert!(shown("", "full"));
        assert!(!shown("", "agent"));
        assert!(!shown(r#"info_lines = "auto""#, "compact"));
        assert!(shown(r#"info_lines = "true""#, "compact"));
        assert!(shown("info_lines = True", "silent"));
        assert!(!shown(r#"info_lines = "false""#, "full"));
        assert!(!shown("info_lines = False", "plain"));
        let err = resolve_with("full", "cli", "info_lines = 1", false).unwrap_err();
        assert!(err.contains("info_lines must be"), "{err}");
    }

    #[test]
    fn the_surface_carries_near_miss_warnings() {
        crate::test::eval(r#"task_presets = {"chek": "silent", "release-notes": "silent"}"#)
            .with_value("task_presets", |task_presets| {
                let args = Arguments::new();
                args.insert("task_presets".into(), task_presets);
                let paths = ["check".to_owned()];
                let surface = from_feature_args(&args, task(), Host::default(), &paths).unwrap();
                assert_eq!(
                    surface.warnings(),
                    [r#"Console task_presets["chek"] names no task here; did you mean "check"?"#]
                );
                assert!(
                    from_feature_args(&args, task(), Host::default(), &[])
                        .unwrap()
                        .warnings()
                        .is_empty()
                );
            });
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
    fn std_streams_report_the_run_color() {
        let code = r#"
def _impl(ctx):
    io = ctx.std.io
    return (2 if io.stderr.color else 0) + (1 if io.stdout.color else 0)

check = task(implementation = _impl)
"#;
        let color = |stdout_tty, stderr_tty, vars: &[(&str, &str)]| {
            crate::test::eval(code)
                .with_host_env(stdout_tty, stderr_tty, vars)
                .run_task(0)
                .unwrap()
        };
        assert_eq!(color(true, true, &[]), Some(3), "terminals");
        assert_eq!(
            color(false, false, &[("GITHUB_ACTIONS", "true")]),
            Some(2),
            "an ANSI-rendering CI host colors stderr only"
        );
        assert_eq!(color(false, false, &[]), Some(0), "pipes");
        assert_eq!(
            color(true, false, &[("BUILDKITE", "true"), ("NO_COLOR", "1")]),
            Some(0),
            "NO_COLOR"
        );
    }

    #[test]
    fn a_buildkite_snapshot_opens_sections() {
        let code = r#"
def _impl(ctx):
    line = ctx.console.render("phase", title = "Build", emoji = "", highlight_style = "", highlight_reset = "", text = "")
    return 1 if line.startswith(":aspect:") else 0

check = task(implementation = _impl)
"#;
        let on = |vars: &[(&str, &str)]| {
            crate::test::eval(code)
                .with_host_env(false, false, vars)
                .run_task(0)
                .unwrap()
        };
        assert_eq!(on(&[("BUILDKITE", "true")]), Some(1), "Buildkite");
        assert_eq!(on(&[("BUILDKITE", "")]), Some(0), "an empty BUILDKITE");
        assert_eq!(on(&[]), Some(0), "elsewhere");
    }

    #[test]
    fn an_agent_snapshot_picks_the_agent_preset() {
        let code = r#"
def _feature(ctx):
    pass

def _impl(ctx):
    return 1 if ctx.console.template_preset == "agent" else 0

Console = feature(
    implementation = _feature,
    args = {"template_preset": args.string(default = "full")},
)
check = task(implementation = _impl)
"#;
        let on = |vars: &[(&str, &str)]| {
            crate::test::eval(code)
                .with_features(&["Console"])
                .with_host_env(false, false, vars)
                .run_task(0)
                .unwrap()
        };
        assert_eq!(on(&[("CLAUDECODE", "1")]), Some(1), "Claude Code");
        assert_eq!(on(&[("AI_AGENT", "")]), Some(0), "an empty AI_AGENT");
        assert_eq!(on(&[]), Some(0), "no agent");
    }

    #[test]
    fn a_file_stream_never_reports_color() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.txt");
        let code = format!(
            r#"
def _impl(ctx):
    f = ctx.std.fs.create({path:?})
    color = f.color
    f.close()
    return 1 if color else 0

check = task(implementation = _impl)
"#,
            path = path.to_str().unwrap()
        );
        let got = crate::test::eval(&code)
            .with_host_env(true, true, &[])
            .run_task(0)
            .unwrap();
        assert_eq!(got, Some(0));
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
            format!("{err:?}").contains(r#"unknown slot "verdcit"; did you mean "verdict"?"#),
            "{err:?}"
        );
    }
}
