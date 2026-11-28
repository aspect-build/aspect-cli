//! The Starlark side of [`crate::console`]: reading the `Console` feature's
//! args into a resolved surface for a given [`Host`], and `ctx.console`,
//! through which AXL renders the `phase`, `progress`, `section` and `tip`
//! lines and asks whether the run opens Buildkite log sections.

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

use crate::diag::Verbose;

use super::arguments::Arguments;
use super::template::json_to_minijinja;

/// The built-in tasks with a preset of their own, by command path, as if
/// `task_presets` named them: `aspect history browse` is its full screen
/// alone, and `aspect history list` and `show` print only what they show
/// (for a script to read), with no header or verdict around either. A
/// `task_presets` entry for
/// the task, by its command path or its kind, replaces one; a
/// `task_templates` entry's slots beat it, as they beat any task's preset.
/// Here rather than as the `Console` arg's default, which an `args.custom`
/// dict cannot have.
const BUILTIN_TASK_PRESETS: &[(&str, &str)] = &[
    ("history browse", "silent"),
    ("history list", "silent"),
    ("history show", "silent"),
];

/// Resolve the surface for `task` on `host` from the `Console` feature's
/// merged `args`.
///
/// `template_preset` counts only when the command line or a `config.axl` set
/// it; otherwise the base preset is `agent` when [`Host::coding_agent`] and
/// `full` when not. The verbose level is the `verbose` arg when the command
/// line set it, else [`Host::verbose`] (`ASPECT_DEBUG` / `ASPECT_VERBOSE`),
/// else the arg from a `config.axl`, else the preset's. `auto` leaves it to
/// the next, except that `auto` on the command line replaces the arg a
/// `config.axl` set, so it skips to the preset when the environment sets
/// none. A disabled feature gives the `full` layout and the environment's
/// level. It also sets what links to files open, for the process
/// (`file_hyperlink_format`, see `crate::out::set_link_format`), whether or not
/// the feature is enabled; `config_editor` is the `editor` of the user's
/// `~/.aspect/config.json`, else of the repository's `.aspect/config.json`
/// (`crate::out::config_link_format`), which beats a `config.axl`'s arg and
/// loses to the command line and the environment.
/// Fails on a malformed `templates`, `task_presets`, `task_templates`,
/// `verbose` or `file_hyperlink_format` value and on everything
/// [`Surface::resolve`] refuses. The surface carries a
/// warning for each `task_presets` / `task_templates` key that is a near miss
/// of one of `task_paths`, the command paths of the repo's tasks.
pub fn from_feature_args(
    args: &Arguments<'_>,
    task: TaskIdentity,
    host: Host,
    task_paths: &[String],
    config_editor: Option<Option<bazel::console::links::LinkFormat>>,
) -> Result<Surface, String> {
    crate::out::set_link_format(file_hyperlink_format(
        args,
        crate::out::env_link_format(),
        config_editor,
    )?);
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
    let verbose_arg = verbose(args)?;
    let level = if args.is_explicit_key("verbose") {
        verbose_arg.or(host.verbose)
    } else {
        host.verbose.or(verbose_arg)
    };
    if let Some(level) = level {
        overrides.set_verbose(level);
    }
    let templates = string_dict(args, "templates")?;
    blocks_error(&templates)?;
    for (slot, template) in templates {
        overrides.set(
            slot,
            string_value(template, || format!("templates[{slot:?}]"))?,
        )?;
    }
    let presets = string_dict(args, "task_presets")?;
    for (path, name) in BUILTIN_TASK_PRESETS {
        let kind = path.rsplit(' ').next().unwrap_or(path);
        if !presets.iter().any(|(key, _)| *key == *path || *key == kind) {
            overrides.set_preset_for_task(path, (*name).to_owned())?;
        }
    }
    for (task_key, name) in presets {
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

/// What a link to a file opens, `None` for `auto` (`LinkFormat::auto`):
/// the `file_hyperlink_format` arg typed on the command line, else `env`
/// (`ASPECT_FILE_HYPERLINK_FORMAT`, see `crate::out::env_link_format`), else
/// `config` (the `editor` of the user's `~/.aspect/config.json`, else of the
/// repository's `.aspect/config.json`, see `crate::out::config_link_format`),
/// else the arg a `config.axl` set, else `auto`. Fails on an invalid arg or
/// `env`; an invalid `editor` was warned about and reaches here as unset.
fn file_hyperlink_format(
    args: &Arguments<'_>,
    env: Option<Result<Option<bazel::console::links::LinkFormat>, String>>,
    config: Option<Option<bazel::console::links::LinkFormat>>,
) -> Result<Option<bazel::console::links::LinkFormat>, String> {
    let arg = || match args
        .get("file_hyperlink_format")
        .and_then(|v| v.unpack_str())
    {
        None | Some("auto") => Ok(None),
        Some(value) => bazel::console::links::LinkFormat::parse(value)
            .map(Some)
            .map_err(|e| format!("Console file_hyperlink_format: {e}")),
    };
    if args.is_explicit_key("file_hyperlink_format") {
        return arg();
    }
    if let Some(chosen) = env {
        return chosen;
    }
    match config {
        Some(chosen) => Ok(chosen),
        None => arg(),
    }
}

/// The `verbose` arg: `None` for `auto` or unset, so the preset decides.
/// The CLI gives a string; a `config.axl` may assign a bool too.
fn verbose(args: &Arguments<'_>) -> Result<Option<Verbose>, String> {
    let Some(value) = args.get("verbose").filter(|v| !v.is_none()) else {
        return Ok(None);
    };
    if let Some(show) = value.unpack_bool() {
        return Ok(Some(if show { Verbose::On } else { Verbose::Off }));
    }
    let given = value.unpack_str();
    given
        .and_then(|name| Verbose::parse(name).ok())
        .ok_or_else(|| {
            format!(
                "Console verbose must be \"auto\", \"true\", \"false\", \"debug\" or a bool, got {}{}",
                value.to_repr(),
                given
                    .map(|g| crate::suggest::did_you_mean(g, Verbose::NAMES))
                    .unwrap_or_default()
            )
        })
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
    /// The preset this task renders with (`full`, `emoji`, `compact`,
    /// `agent` or `silent`): its `task_presets` entry, else the `Console`
    /// feature's `template_preset`, which wins outright when typed on the
    /// command line. With neither set, `agent` under a coding agent and `full`
    /// otherwise. For task output that should follow it.
    #[starlark(attribute)]
    fn template_preset<'v>(this: Value<'v>) -> anyhow::Result<&'static str> {
        Ok(downcast(this)?.0.preset())
    }

    /// Whether this run is on Buildkite, where the console opens log sections
    /// (`--- <title>`): the value the templates see as `buildkite`, resolved
    /// once for the run. Ask it before printing a section header of your own.
    #[starlark(attribute)]
    fn buildkite<'v>(this: Value<'v>) -> anyhow::Result<bool> {
        Ok(downcast(this)?.0.buildkite())
    }

    /// Render one slot (`banner`, `header`, `phase`, `progress`, `verdict`,
    /// `section` or `tip`) with the kwargs as its context. Returns `""` when the
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
    use bazel::console::links::LinkFormat;

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
        render_as(code, slot, &task())
    }

    /// [`render`] for the task `who`.
    fn render_as(code: &str, slot: Slot, who: &TaskIdentity) -> Result<String, String> {
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
            let surface = from_feature_args(&args, who.clone(), Host::default(), &[], None)?;
            Ok(surface.render(slot, minijinja::context! { text => "x" }))
        })
    }

    /// `aspect history browse` takes its built-in preset (`silent`: no
    /// header or verdict), unless the user's own entry for it, by kind or
    /// command path, says otherwise.
    #[test]
    fn browse_takes_its_builtin_preset_unless_the_user_names_it() {
        let browse = TaskIdentity {
            kind: "browse".into(),
            group: vec!["history".into()],
            name: "browse".into(),
            path: "history browse".into(),
        };
        let at = |code: &str, slot| render_as(code, slot, &browse).unwrap();
        assert_eq!(at("", Slot::Verdict), "");
        assert_eq!(at("", Slot::Header), "");
        assert_eq!(
            at(
                r#"task_templates = {"browse": {"verdict": "V {{ text }}"}}"#,
                Slot::Verdict
            ),
            "V x"
        );
        assert_ne!(
            at(r#"task_presets = {"history browse": "full"}"#, Slot::Header),
            ""
        );
        assert_ne!(at(r#"task_presets = {"browse": "full"}"#, Slot::Header), "");
        for (kind, path) in [("list", "history list"), ("show", "history show")] {
            let task = TaskIdentity {
                kind: kind.into(),
                group: vec!["history".into()],
                name: kind.into(),
                path: path.into(),
            };
            assert_eq!(render_as("", Slot::Verdict, &task).unwrap(), "", "{path}");
        }
        assert_ne!(
            render(r#"task_presets = {"lint": "full"}"#, Slot::Header).unwrap(),
            "",
            "another task keeps its own"
        );
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
        assert_eq!(render("", Slot::Progress).unwrap(), "▹ x");
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
                "Console templates takes slot keys only (banner, header, phase, progress, verdict, section, tip); \
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
            let surface = from_feature_args(&args, task(), Host::default(), &[], None).unwrap();
            assert_eq!(surface.preset(), "full");
        });
        let surface =
            from_feature_args(&Arguments::new(), task(), Host::default(), &[], None).unwrap();
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
        resolve_on(preset, source, extra, "cli", coding_agent, None)
    }

    /// [`resolve_with`], with `verbose` marked as `verbose_source` set it
    /// (as `source` does for `template_preset`) and `env` as the
    /// environment's verbose level.
    fn resolve_on(
        preset: &str,
        source: &str,
        extra: &str,
        verbose_source: &str,
        coding_agent: bool,
        env: Option<Verbose>,
    ) -> Result<Surface, String> {
        let code = format!(
            "verbose = None\n{extra}\nargs = {{\"template_preset\": {preset:?}, \"verbose\": verbose}}"
        );
        crate::test::eval(&code).with_value("args", |bound| {
            let args = Arguments::new();
            for (name, value) in DictRef::from_value(bound).unwrap().iter() {
                args.insert(name.unpack_str().unwrap().into(), value);
            }
            for (key, source) in [("template_preset", source), ("verbose", verbose_source)] {
                match source {
                    "cli" => args.mark_explicit(key.into()),
                    "config" => args.mark_configured(key.into()),
                    _ => {}
                }
            }
            let host = Host {
                coding_agent,
                verbose: env,
                ..Host::default()
            };
            from_feature_args(&args, task(), host, &[], None)
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
            preset("emoji", "cli", true),
            "emoji",
            "the CLI beats detection"
        );
        assert_eq!(preset("compact", "config", false), "compact");
    }

    #[test]
    fn verbose_overrides_the_preset() {
        let shown = |extra, preset| resolve_with(preset, "cli", extra, false).unwrap().verbose();
        assert!(shown("", "full"));
        assert!(!shown("", "agent"));
        assert!(!shown(r#"verbose = "auto""#, "compact"));
        assert!(shown(r#"verbose = "true""#, "compact"));
        assert!(shown("verbose = True", "silent"));
        assert!(!shown(r#"verbose = "false""#, "full"));
        assert!(!shown("verbose = False", "emoji"));
        let err = resolve_with("full", "cli", "verbose = 1", false).unwrap_err();
        assert!(err.contains("verbose must be"), "{err}");
        let err = resolve_with("full", "cli", r#"verbose = "ture""#, false).unwrap_err();
        assert!(
            err.ends_with(r#"got "ture"; did you mean "true"?"#),
            "{err}"
        );
        let err = resolve_with("full", "cli", r#"verbose = "debgu""#, false).unwrap_err();
        assert!(
            err.ends_with(r#"got "debgu"; did you mean "debug"?"#),
            "{err}"
        );
    }

    #[test]
    fn verbose_takes_the_command_line_then_the_environment_then_config() {
        let level = |extra, source, env| {
            resolve_on("full", "cli", extra, source, false, env)
                .unwrap()
                .verbosity()
        };
        let debug = r#"verbose = "debug""#;
        let off = "verbose = False";
        assert_eq!(level(debug, "cli", None), Verbose::Debug);
        assert_eq!(
            level(off, "cli", Some(Verbose::Debug)),
            Verbose::Off,
            "CLI > env"
        );
        assert_eq!(
            level(off, "config", Some(Verbose::Debug)),
            Verbose::Debug,
            "env > config"
        );
        assert_eq!(
            level(debug, "config", None),
            Verbose::Debug,
            "config > preset"
        );
        assert_eq!(
            level("", "", Some(Verbose::Debug)),
            Verbose::Debug,
            "env > preset"
        );
        assert_eq!(level("", "", None), Verbose::On, "the preset's");
        assert_eq!(
            level(r#"verbose = "auto""#, "cli", Some(Verbose::Off)),
            Verbose::Off,
            "auto on the CLI leaves it to the environment"
        );
        assert_eq!(
            level(r#"verbose = "auto""#, "cli", None),
            Verbose::On,
            "auto on the CLI, which replaced config's arg, leaves it to the preset"
        );
    }

    /// [`file_hyperlink_format`] with the arg bound to `value` and marked as
    /// `source` set it (`"cli"`, `"config"`, or `""` for the schema default).
    fn link_format_from(
        value: &str,
        source: &str,
        env: Option<Result<Option<LinkFormat>, String>>,
        user: Option<Option<LinkFormat>>,
    ) -> Result<Option<LinkFormat>, String> {
        crate::test::eval(&format!("value = {value:?}")).with_value("value", |bound| {
            let args = Arguments::new();
            args.insert("file_hyperlink_format".into(), bound);
            match source {
                "cli" => args.mark_explicit("file_hyperlink_format".into()),
                "config" => args.mark_configured("file_hyperlink_format".into()),
                _ => {}
            }
            file_hyperlink_format(&args, env, user)
        })
    }

    #[test]
    fn file_hyperlink_format_takes_the_command_line_then_env_then_user_then_config() {
        let format = |name| Some(LinkFormat::parse(name).unwrap());
        let zed = Some(format("zed"));
        assert_eq!(
            link_format_from("cursor", "cli", Some(Ok(format("idea"))), zed.clone()),
            Ok(format("cursor")),
            "CLI > env, user"
        );
        assert_eq!(
            link_format_from("auto", "cli", None, zed.clone()),
            Ok(None),
            "auto on the CLI is a choice"
        );
        assert_eq!(
            link_format_from("cursor", "config", Some(Ok(format("idea"))), zed.clone()),
            Ok(format("idea")),
            "env > user, config"
        );
        assert_eq!(
            link_format_from("cursor", "config", None, zed.clone()),
            Ok(format("zed")),
            "user > config"
        );
        assert_eq!(
            link_format_from("cursor", "config", None, Some(None)),
            Ok(None),
            "the user's auto beats config"
        );
        assert_eq!(
            link_format_from("cursor", "config", None, None),
            Ok(format("cursor")),
            "config with no user editor"
        );
        assert_eq!(
            link_format_from("auto", "", None, None),
            Ok(None),
            "auto by default"
        );
        assert_eq!(
            link_format_from("bogus", "config", None, zed),
            Ok(format("zed")),
            "an invalid config arg the user's editor beats is not read"
        );
        let err = link_format_from("bogus", "config", None, None).unwrap_err();
        assert!(err.starts_with("Console file_hyperlink_format: "), "{err}");
    }

    #[test]
    fn the_surface_carries_near_miss_warnings() {
        crate::test::eval(r#"task_presets = {"chek": "silent", "release-notes": "silent"}"#)
            .with_value("task_presets", |task_presets| {
                let args = Arguments::new();
                args.insert("task_presets".into(), task_presets);
                let paths = ["check".to_owned()];
                let surface =
                    from_feature_args(&args, task(), Host::default(), &paths, None).unwrap();
                assert_eq!(
                    surface.warnings(),
                    [r#"Console task_presets["chek"] names no task here; did you mean "check"?"#]
                );
                assert!(
                    from_feature_args(&args, task(), Host::default(), &[], None)
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
    if line != "▹ x":
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
        assert_eq!(
            color(true, true, &[("TERM", "xterm-256color")]),
            Some(3),
            "terminals"
        );
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

    /// `ctx.console.buildkite` and the section titles follow the same host
    /// snapshot: on Buildkite a phase is its section's title, with no glyph.
    #[test]
    fn a_buildkite_snapshot_opens_sections() {
        let code = r#"
def _impl(ctx):
    line = ctx.console.render("phase", title = "Build", emoji = "", highlight_style = "", highlight_reset = "", text = "")
    return (2 if ctx.console.buildkite else 0) + (1 if "Build" in line and "\u25b8" not in line else 0)

check = task(implementation = _impl)
"#;
        let on = |vars: &[(&str, &str)]| {
            crate::test::eval(code)
                .with_host_env(false, false, vars)
                .run_task(0)
                .unwrap()
        };
        assert_eq!(on(&[("BUILDKITE", "true")]), Some(3), "Buildkite");
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
            .with_host_env(true, true, &[("TERM", "xterm-256color")])
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
