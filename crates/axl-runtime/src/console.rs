//! The terminal status surface: the lines Aspect itself prints around a task.
//!
//! Six slots cover them — `banner`, `header`, `phase`, `progress`, `verdict`
//! and `section` — each a Jinja2 template rendered with the context documented
//! on [`Slot`]. The runtime renders `banner`, `header` and `verdict`; AXL
//! renders the rest through `ctx.console.render(...)` (see
//! `lib/lifecycle.axl`). Bazel's own output, `warn()` / `error()` diagnostics
//! and stdout never pass through here.
//!
//! The `Console` feature (`feature/console.axl`) selects the templates the same
//! way the CI status surfaces do, through four args: `template_preset` names a
//! built-in [`Preset`], `templates` overrides single slots on top of it for
//! every task, and `task_presets` and `task_templates` do the same for one
//! task, keyed by task kind (`lint`) or command path (`worktree add`), the
//! command path winning. A task with its own preset starts from it alone,
//! without `templates`, unless `template_preset` was typed on the command
//! line, which applies it to every task. Layers resolve in order, so a preset
//! never outranks the config that selected it:
//!
//! ```text
//! task_presets[<task>], else template_preset + templates
//!   <  task_templates[<task kind>]  <  task_templates[<command path>]
//! ```
//!
//! A key that names no task here is inert, so a user-level `config.axl` can
//! name tasks only some repos have; one that is a near miss of a task here
//! (`tset`) gets a warning naming the task it likely meant.
//!
//! Two rules keep a template from costing the reader the task's outcome or the
//! log's structure:
//!
//! - An override that fails to render falls back to the selected preset's
//!   template for that slot, with one warning per slot.
//! - On Buildkite, `header`, `phase`, `verdict` and `section` open log
//!   sections. The section marker is printed for them, leading whitespace is
//!   dropped so it is never untitled, and a blank render is replaced by the
//!   preset's text (or `full`'s), so collapsing and the failing-phase
//!   expansion survive every layout.
//!
//! Overrides are compiled when the console is resolved, so a syntax error fails
//! the run before the task starts rather than mid-task.
//!
//! A console never reads the process: whether it colors and whether it opens
//! Buildkite sections come from the [`Host`] it is built with, which a run
//! takes from its [`Env`] ([`Host::of`]) and a test spells out.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Mutex;

use minijinja::{Environment, Value as JinjaValue};

use crate::engine::store::Env;

/// One line-shaped piece of the terminal surface.
///
/// Every slot's context also carries `buildkite` (bool) and `task`
/// (`kind`, `group`, `name`, `path`). Where a slot has `highlight_style`, it
/// is the ANSI escape that opens the line's highlight and `highlight_reset`
/// the one that closes it; both are empty when color is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Slot {
    /// Identity line above the header, shown only on a TTY or a recognized CI
    /// host. Context: `line` (the whole built-in line, pre-styled), `version`,
    /// `debug_build` (bool), `docs_url`, `highlight_style`, `highlight_reset`.
    Banner,
    /// Opening line. Context: `label`, `highlight_style`, `highlight_reset`.
    Header,
    /// A phase entry (off Buildkite) or a phase's section (on Buildkite).
    /// Context: `title`, `emoji`, `highlight_style`, `highlight_reset`,
    /// `text`.
    Phase,
    /// A mid-phase update. Context: `text`; `title`, `emoji`,
    /// `highlight_style`, `highlight_reset` when the update names its phase.
    Progress,
    /// Closing line, styled in the verdict's color. Context: `verdict`
    /// (`passed` | `flagged` | `failed`), `glyph`, `shortcode`, `verb`,
    /// `highlight_style`, `highlight_reset`, `label`, `exit_code`,
    /// `exit`, `elapsed`, `elapsed_ms`, `timing`, `conclusion`, `breakdown`,
    /// `plain_breakdown` (the same without phase emojis),
    /// and `phases` (every recorded phase, the synthetic `init` included:
    /// `name`, `title`, `emoji`, `description`, `duration`, `duration_ms`,
    /// `interrupted`).
    Verdict,
    /// A Buildkite log section a task or feature opens itself, such as the
    /// Workflows runner metadata; only printed on Buildkite. Context: `icon`
    /// (a Buildkite emoji shortcode, or empty), `title`.
    Section,
}

impl Slot {
    pub const ALL: [Slot; 6] = [
        Slot::Banner,
        Slot::Header,
        Slot::Phase,
        Slot::Progress,
        Slot::Verdict,
        Slot::Section,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Slot::Banner => "banner",
            Slot::Header => "header",
            Slot::Phase => "phase",
            Slot::Progress => "progress",
            Slot::Verdict => "verdict",
            Slot::Section => "section",
        }
    }

    pub fn from_name(name: &str) -> Option<Slot> {
        Slot::ALL.into_iter().find(|s| s.name() == name)
    }

    fn opens_buildkite_section(self) -> bool {
        matches!(
            self,
            Slot::Header | Slot::Phase | Slot::Verdict | Slot::Section
        )
    }

    fn parse(name: &str, at: &str) -> Result<Slot, String> {
        Slot::from_name(name).ok_or_else(|| {
            format!(
                "Console {at}: {name:?} is not a slot; expected one of {}",
                Slot::ALL.map(Slot::name).join(", ")
            )
        })
    }
}

/// A named built-in layout: one template per [`Slot`]. The `Console` feature's
/// `template_preset` arg lists the same names and describes each.
pub struct Preset {
    pub name: &'static str,
    banner: &'static str,
    header: &'static str,
    phase: &'static str,
    progress: &'static str,
    verdict: &'static str,
    section: &'static str,
}

impl Preset {
    fn template(&self, slot: Slot) -> &'static str {
        match slot {
            Slot::Banner => self.banner,
            Slot::Header => self.header,
            Slot::Phase => self.phase,
            Slot::Progress => self.progress,
            Slot::Verdict => self.verdict,
            Slot::Section => self.section,
        }
    }
}

/// The verdict after its leading `→ ` (off Buildkite) and its icon, up to the
/// phase breakdown.
macro_rules! verdict_text {
    () => {
        concat!(
            "{{ highlight_style }}{{ verb }}{{ highlight_reset }} ",
            "{% if buildkite %}· {% endif %}{{ label }}{{ exit }}{{ timing }}",
            "{% if conclusion %} · {{ conclusion }}{% endif %}",
        )
    };
}

/// The header after its icon.
macro_rules! header_text {
    () => {
        "{{ highlight_style }}Running{{ highlight_reset }} {{ label }}"
    };
}

/// The verdict's icon: its glyph off Buildkite, its emoji shortcode on it.
macro_rules! verdict_icon {
    () => {
        "{{ shortcode if buildkite else glyph }} "
    };
}

/// The layout Aspect has always printed, and a blank Buildkite section's last
/// fallback.
///
/// Off Buildkite a phase line is set off by a blank line and its emoji sits
/// inside the highlight; on Buildkite the line is a section title, so the
/// emoji leads it and a phase without one shows `:aspect:`.
const FULL: Preset = Preset {
    name: "full",
    // Jinja drops one trailing newline, so two keep the blank line under it.
    banner: "{{ line }}\n\n",
    header: concat!("{{ ':aspect:' if buildkite else '→ 🎬' }} ", header_text!()),
    phase: concat!(
        "{% if not buildkite %}\n→ {{ highlight_style }}{% endif %}",
        "{% if emoji or buildkite %}{{ emoji or ':aspect:' }} {% endif %}",
        "{% if buildkite %}{{ highlight_style }}{% endif %}",
        "{{ title }}{{ highlight_reset }}{% if text %} · {{ text }}{% endif %}",
    ),
    progress: "→ {% if title %}{{ highlight_style }}{% if emoji %}{{ emoji }} {% endif %}{{ title }}{{ highlight_reset }} · {% endif %}{{ text }}",
    verdict: concat!(
        "{% if not buildkite %}\n→ {% endif %}",
        verdict_icon!(),
        verdict_text!(),
        "{{ breakdown }}"
    ),
    section: "{% if icon %}{{ icon }} {% endif %}{{ title }}",
};

/// `full` without emojis or Buildkite emoji shortcodes.
const PLAIN: Preset = Preset {
    name: "plain",
    banner: FULL.banner,
    header: concat!("{% if not buildkite %}→ {% endif %}", header_text!()),
    phase: concat!(
        "{% if not buildkite %}\n→ {% endif %}",
        "{{ highlight_style }}{{ title }}{{ highlight_reset }}{% if text %} · {{ text }}{% endif %}",
    ),
    progress: "→ {% if title %}{{ highlight_style }}{{ title }}{{ highlight_reset }} · {% endif %}{{ text }}",
    verdict: concat!(
        "{% if not buildkite %}\n→ {% endif %}",
        verdict_text!(),
        "{{ plain_breakdown }}"
    ),
    section: "{{ title }}",
};

const COMPACT: Preset = Preset {
    name: "compact",
    banner: "",
    header: "",
    phase: "",
    progress: "",
    verdict: concat!(
        "{% if not buildkite %}→ {% endif %}",
        verdict_icon!(),
        verdict_text!()
    ),
    section: "",
};

const SILENT: Preset = Preset {
    name: "silent",
    banner: "",
    header: "",
    phase: "",
    progress: "",
    verdict: "",
    section: "",
};

/// Built-in presets, in the order `--help` lists them.
pub const PRESETS: [&Preset; 4] = [&FULL, &PLAIN, &COMPACT, &SILENT];

/// The preset a run uses when nothing selects another.
pub const DEFAULT_PRESET: &str = FULL.name;

/// The `Console` feature's `templates`, `task_presets` and `task_templates`,
/// before they are narrowed to one task.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    templates: BTreeMap<Slot, String>,
    /// Keyed by a task kind or command path, as is `task_presets`.
    task_templates: BTreeMap<String, BTreeMap<Slot, String>>,
    task_presets: BTreeMap<String, String>,
    /// The feature's preset came from the command line, so it beats
    /// `task_presets`.
    preset_from_cli: bool,
}

impl Overrides {
    /// Override `slot` (a slot name) for every task.
    pub fn set(&mut self, slot: &str, template: String) -> Result<(), String> {
        let slot = Slot::parse(slot, "templates")?;
        self.templates.insert(slot, template);
        Ok(())
    }

    /// Override `slot` (a slot name) for the tasks `task` names: a task kind
    /// (`lint`) or a command path (`worktree add`).
    pub fn set_for_task(&mut self, task: &str, slot: &str, template: String) -> Result<(), String> {
        let label = format!("task_templates[{task:?}]");
        task_key(task, &label)?;
        let slot = Slot::parse(slot, &label)?;
        self.task_templates
            .entry(task.to_owned())
            .or_default()
            .insert(slot, template);
        Ok(())
    }

    /// Start the tasks `task` names from the preset `name`, in place of the
    /// feature's preset and `templates`. The name is checked when the console
    /// is resolved.
    pub fn set_preset_for_task(&mut self, task: &str, name: String) -> Result<(), String> {
        task_key(task, &format!("task_presets[{task:?}]"))?;
        self.task_presets.insert(task.to_owned(), name);
        Ok(())
    }

    /// Apply the feature's preset to every task, over `task_presets`: it was
    /// typed on the command line.
    pub fn preset_from_cli(&mut self) {
        self.preset_from_cli = true;
    }

    /// Every override with the arg path naming it.
    fn iter(&self) -> impl Iterator<Item = (String, &str)> {
        let flat = self
            .templates
            .iter()
            .map(|(slot, t)| (format!("templates[{:?}]", slot.name()), t.as_str()));
        let scoped = self.task_templates.iter().flat_map(|(task, block)| {
            block.iter().map(move |(slot, t)| {
                (
                    format!("task_templates[{task:?}][{:?}]", slot.name()),
                    t.as_str(),
                )
            })
        });
        flat.chain(scoped)
    }

    /// `task`'s own preset, if any, and the overrides it renders with:
    /// `templates` unless it has its own preset, then its kind's
    /// `task_templates`, then its command path's.
    fn for_task(&self, task: &TaskIdentity) -> (Option<&str>, BTreeMap<Slot, String>) {
        let preset = self.preset_for(task);
        let mut resolved = match preset {
            Some(_) => BTreeMap::new(),
            None => self.templates.clone(),
        };
        for key in task.scope_keys() {
            if let Some(block) = self.task_templates.get(key) {
                resolved.extend(block.iter().map(|(slot, t)| (*slot, t.clone())));
            }
        }
        (preset, resolved)
    }

    /// A warning for each `task_presets` / `task_templates` key that names no
    /// task here but is a near miss of one that does (see
    /// [`closest_task_key`]), naming every arg the key appears in. A key far
    /// from every task stays silent: a user-level `config.axl` applies to
    /// every repo, and only some of them define the task.
    pub fn near_miss_warnings(&self, task_paths: &[String]) -> Vec<String> {
        let keys: BTreeSet<&str> = self
            .task_presets
            .keys()
            .chain(self.task_templates.keys())
            .map(String::as_str)
            .collect();
        keys.into_iter()
            .filter_map(|key| {
                let closest = closest_task_key(key, task_paths)?;
                let args: Vec<String> = [
                    ("task_presets", self.task_presets.contains_key(key)),
                    ("task_templates", self.task_templates.contains_key(key)),
                ]
                .into_iter()
                .filter(|(_, has)| *has)
                .map(|(arg, _)| format!("{arg}[{key:?}]"))
                .collect();
                let verb = if args.len() == 1 { "names" } else { "name" };
                Some(format!(
                    "Console {} {verb} no task here; did you mean {closest:?}?",
                    args.join(" and ")
                ))
            })
            .collect()
    }

    /// `task`'s `task_presets` entry, the command path's over the kind's.
    fn preset_for(&self, task: &TaskIdentity) -> Option<&str> {
        if self.preset_from_cli {
            return None;
        }
        task.scope_keys()
            .rev()
            .find_map(|key| self.task_presets.get(key))
            .map(String::as_str)
    }
}

/// Identity of the running task, shared by every slot's context.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TaskIdentity {
    pub kind: String,
    pub group: Vec<String>,
    pub name: String,
    /// `<group…> <kind>`, as typed on the command line.
    pub path: String,
}

impl TaskIdentity {
    /// The keys `task_presets` and `task_templates` name this task by, least
    /// specific first: its kind, then its command path when that differs.
    fn scope_keys(&self) -> impl DoubleEndedIterator<Item = &str> {
        let path = (self.path != self.kind).then_some(self.path.as_str());
        std::iter::once(self.kind.as_str()).chain(path)
    }
}

/// What the console takes from the host it runs on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Host {
    /// Whether the console lines may carry ANSI color: the run's
    /// `std.io.stderr.color`, so every slot, whether the runtime or AXL
    /// renders it, agrees with AXL's own stderr lines.
    pub color: bool,
    /// Whether the run is on Buildkite, where the section slots open log
    /// sections.
    pub buildkite: bool,
}

impl Host {
    /// The host of the run `env` describes: its stderr color and its
    /// Buildkite snapshot, both resolved once for the run (the process's
    /// own, or a test harness's).
    pub fn of(env: &Env) -> Host {
        Host {
            color: env.stream_color.stderr,
            buildkite: env.buildkite,
        }
    }
}

/// The resolved surface for one task run.
#[derive(Debug)]
pub struct Console {
    preset: &'static Preset,
    overrides: BTreeMap<Slot, String>,
    host: Host,
    task: TaskIdentity,
    warned: Mutex<HashSet<Slot>>,
}

impl std::fmt::Debug for Preset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

impl Console {
    /// The `full` layout with no overrides, on `host`: what a run without the
    /// `Console` feature (or with it disabled) prints.
    pub fn builtin(task: TaskIdentity, host: Host) -> Console {
        Console::new(&FULL, BTreeMap::new(), task, host)
    }

    /// Resolve `preset_name` plus `overrides` for `task`, on `host`.
    ///
    /// Fails on an unknown preset or an override that does not compile,
    /// wherever it is scoped. An entry for a task this repo lacks is checked but
    /// otherwise inert, as on the CI surfaces: a user-level `config.axl` applies
    /// to every repo, and only some of them define that task. A near miss of a
    /// task here is warned about by [`Overrides::near_miss_warnings`].
    pub fn resolve(
        preset_name: &str,
        overrides: &Overrides,
        task: TaskIdentity,
        host: Host,
    ) -> Result<Console, String> {
        for (key, name) in &overrides.task_presets {
            preset_named(name, &format!("task_presets[{key:?}]"))?;
        }
        let (own_preset, resolved) = overrides.for_task(&task);
        let preset = preset_named(own_preset.unwrap_or(preset_name), "template_preset")?;
        let env = Environment::new();
        for (at, template) in overrides.iter() {
            env.template_from_str(template)
                .map_err(|e| format!("Console {at} does not compile: {e}"))?;
        }
        Ok(Console::new(preset, resolved, task, host))
    }

    fn new(
        preset: &'static Preset,
        overrides: BTreeMap<Slot, String>,
        task: TaskIdentity,
        host: Host,
    ) -> Console {
        Console {
            preset,
            overrides,
            host,
            task,
            warned: Mutex::new(HashSet::new()),
        }
    }

    /// Name of the selected preset.
    pub fn preset(&self) -> &'static str {
        self.preset.name
    }

    /// Whether the section slots open Buildkite log sections ([`Host::buildkite`]).
    pub fn buildkite(&self) -> bool {
        self.host.buildkite
    }

    /// Whether the console lines carry ANSI color ([`Host::color`]).
    pub fn color(&self) -> bool {
        self.host.color
    }

    /// Render `slot` with `context`, plus the shared `buildkite` and `task`
    /// keys. Returns `""` for a slot the layout leaves blank, except where
    /// Buildkite needs the line to open a section.
    pub fn render(&self, slot: Slot, context: JinjaValue) -> String {
        let context = minijinja::context! {
            buildkite => self.host.buildkite,
            task => JinjaValue::from_serialize(&self.task),
            ..context
        };
        let rendered = match self.overrides.get(&slot) {
            None => render_builtin(self.preset, slot, &context),
            Some(template) => render_str(template, &context).unwrap_or_else(|e| {
                self.warn_once(slot, &e);
                render_builtin(self.preset, slot, &context)
            }),
        };
        if self.host.buildkite && slot.opens_buildkite_section() {
            return self.section_title(slot, &rendered, &context);
        }
        rendered
    }

    /// `rendered` as the text after a Buildkite section marker. Leading
    /// whitespace, such as the line break a terminal layout uses to set a line
    /// off, would leave the marker untitled, so it goes; a title still blank
    /// after that comes from the preset, or from `full` where the preset leaves
    /// the slot blank.
    fn section_title(&self, slot: Slot, rendered: &str, context: &JinjaValue) -> String {
        let title = rendered.trim_start();
        if !title.is_empty() {
            return title.to_owned();
        }
        [self.preset, &FULL]
            .into_iter()
            .map(|preset| render_builtin(preset, slot, context))
            .find(|text| !text.trim().is_empty())
            .unwrap_or_default()
    }

    fn warn_once(&self, slot: Slot, error: &minijinja::Error) {
        if self
            .warned
            .lock()
            .expect("console warning set")
            .insert(slot)
        {
            crate::diag::warn(&format!(
                "Console template for {:?} failed to render ({error}); using the '{}' layout",
                slot.name(),
                self.preset.name
            ));
        }
    }

    /// Print `slot` to stderr, unless it renders blank. On Buildkite a section
    /// slot opens a collapsed (`---`) section; see [`Console::emit_section`]
    /// for another marker.
    pub fn emit(&self, slot: Slot, context: JinjaValue) {
        if self.host.buildkite && slot.opens_buildkite_section() {
            return self.emit_section("---", slot, context);
        }
        let text = self.render(slot, context);
        if !text.is_empty() {
            crate::errln!("{text}");
        }
    }

    /// Print `slot` as a Buildkite section: `marker`, then the rendered text.
    pub fn emit_section(&self, marker: &str, slot: Slot, context: JinjaValue) {
        let text = self.render(slot, context);
        crate::errln!("{marker} {text}");
    }
}

/// Refuse a `task_presets` / `task_templates` key that is a slot name: it would
/// name no task, so the entry would silently do nothing.
fn task_key(key: &str, label: &str) -> Result<(), String> {
    match Slot::from_name(key) {
        Some(_) => Err(format!(
            "Console {label} names a slot, not a task; key by task kind or command path"
        )),
        None => Ok(()),
    }
}

/// The task key `key` is most likely a typo of, given the command path of
/// every task here (`worktree add`), or `None`.
///
/// A task key is a kind (a path's last word) or a command path. `key` is a
/// near miss when it is no task key itself, is at least three characters (so
/// a short key isn't a couple of edits from everything), and is within two
/// edits of a task key while changing at most half of it. The fewest edits
/// win, then the first key in sorted order. `template_presets.axl` applies
/// the same rule on the CI surfaces.
fn closest_task_key(key: &str, task_paths: &[String]) -> Option<String> {
    let known: BTreeSet<&str> = task_paths
        .iter()
        .flat_map(|path| [path.as_str(), path.rsplit(' ').next().unwrap_or(path)])
        .collect();
    if known.contains(key) || key.chars().count() < 3 {
        return None;
    }
    known
        .into_iter()
        .filter_map(|candidate| {
            let distance = edit_distance(key, candidate);
            (distance <= 2 && 2 * distance <= candidate.chars().count())
                .then_some((distance, candidate))
        })
        .min()
        .map(|(_, candidate)| candidate.to_owned())
}

/// Levenshtein distance between `a` and `b`, in characters.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut current = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
        }
        previous = current;
    }
    previous[b.len()]
}

/// The built-in preset `name`, or an error naming where it was asked for.
fn preset_named(name: &str, at: &str) -> Result<&'static Preset, String> {
    PRESETS.into_iter().find(|p| p.name == name).ok_or_else(|| {
        format!(
            "Console {at}: unknown template preset {name:?}; expected one of {}",
            PRESETS.map(|p| p.name).join(", ")
        )
    })
}

fn render_str(template: &str, context: &JinjaValue) -> Result<String, minijinja::Error> {
    Environment::new().render_str(template, context)
}

/// Built-in templates are covered by this module's tests, so a failure here is
/// a bug in a preset; it renders nothing rather than aborting the task.
fn render_builtin(preset: &Preset, slot: Slot, context: &JinjaValue) -> String {
    render_str(preset.template(slot), context).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> TaskIdentity {
        TaskIdentity {
            kind: "test".into(),
            group: vec![],
            name: "test".into(),
            path: "test".into(),
        }
    }

    fn console(preset: &str, overrides: &Overrides, buildkite: bool) -> Console {
        let host = Host {
            color: false,
            buildkite,
        };
        Console::resolve(preset, overrides, task(), host).unwrap()
    }

    fn verdict(failed: bool) -> JinjaValue {
        minijinja::context! {
            verdict => if failed { "failed" } else { "passed" },
            glyph => if failed { "❌" } else { "✅" },
            shortcode => if failed { ":x:" } else { ":white_check_mark:" },
            verb => if failed { "Failed" } else { "Passed" },
            highlight_style => "", highlight_reset => "",
            label => "test task",
            exit_code => if failed { 3 } else { 0 },
            exit => if failed { " (exit code 3)" } else { "" },
            elapsed => "1.2s", elapsed_ms => 1200,
            timing => " in 1.2s",
            conclusion => "4 tests",
            breakdown => " — 🔨 Build 1.0s",
            plain_breakdown => " — Build 1.0s",
            phases => Vec::<JinjaValue>::new(),
        }
    }

    fn header() -> JinjaValue {
        minijinja::context! { label => "test task", highlight_style => "<c>", highlight_reset => "<r>" }
    }

    fn text(t: &str) -> JinjaValue {
        minijinja::context! { text => t }
    }

    #[test]
    fn full_renders_the_historical_lines() {
        let c = console("full", &Overrides::default(), false);
        assert_eq!(
            c.render(
                Slot::Banner,
                minijinja::context! { line => "Aspect CLI v1" }
            ),
            "Aspect CLI v1\n"
        );
        assert_eq!(
            c.render(Slot::Header, header()),
            "→ 🎬 <c>Running<r> test task"
        );
        assert_eq!(
            c.render(
                Slot::Phase,
                minijinja::context! { title => "Build", emoji => "🔨", highlight_style => "<s>", highlight_reset => "<r>", text => "Building" }
            ),
            "\n→ <s>🔨 Build<r> · Building"
        );
        assert_eq!(c.render(Slot::Progress, text("12 of 40")), "→ 12 of 40");
        assert_eq!(
            c.render(
                Slot::Progress,
                minijinja::context! { title => "Build", emoji => "", highlight_style => "", highlight_reset => "", text => "12 of 40" }
            ),
            "→ Build · 12 of 40"
        );
        assert_eq!(
            c.render(Slot::Verdict, verdict(true)),
            "\n→ ❌ Failed test task (exit code 3) in 1.2s · 4 tests — 🔨 Build 1.0s"
        );
    }

    #[test]
    fn full_renders_the_historical_buildkite_sections() {
        let c = console("full", &Overrides::default(), true);
        assert_eq!(
            c.render(Slot::Header, header()),
            ":aspect: <c>Running<r> test task"
        );
        assert_eq!(
            c.render(
                Slot::Phase,
                minijinja::context! { title => "Build", emoji => "", highlight_style => "", highlight_reset => "", text => "" }
            ),
            ":aspect: Build"
        );
        assert_eq!(
            c.render(
                Slot::Phase,
                minijinja::context! { title => "Build", emoji => "🔨", highlight_style => "<s>", highlight_reset => "<r>", text => "Building" }
            ),
            "🔨 <s>Build<r> · Building"
        );
        assert_eq!(
            c.render(Slot::Verdict, verdict(false)),
            ":white_check_mark: Passed · test task in 1.2s · 4 tests — 🔨 Build 1.0s"
        );
    }

    #[test]
    fn a_task_section_drops_its_icon_only_under_plain() {
        let section = || minijinja::context! { icon => ":rocket:", title => "Runner metadata" };
        let rendered = |preset: &str| {
            console(preset, &Overrides::default(), true).render(Slot::Section, section())
        };
        assert_eq!(rendered("full"), ":rocket: Runner metadata");
        assert_eq!(rendered("plain"), "Runner metadata");
        assert_eq!(
            rendered("silent"),
            ":rocket: Runner metadata",
            "a section is never blank"
        );
        assert_eq!(
            console("full", &Overrides::default(), true).render(
                Slot::Section,
                minijinja::context! { icon => "", title => "Runner metadata" }
            ),
            "Runner metadata"
        );
    }

    #[test]
    fn the_banner_can_be_rebuilt_from_its_pieces() {
        let mut o = Overrides::default();
        o.set(
            "banner",
            "{{ highlight_style }}aspect {{ version }}{% if debug_build %}+debug{% endif %} · {{ docs_url }}{{ highlight_reset }}"
                .into(),
        )
        .unwrap();
        let line =
            console("full", &o, false).render(Slot::Banner, crate::banner::console_context(true));
        assert!(line.starts_with("\x1b[38;5;244maspect "), "{line:?}");
        assert!(
            line.ends_with(" · https://aspect.build/docs/cli\x1b[0m"),
            "{line:?}"
        );
        let full = console("full", &Overrides::default(), false)
            .render(Slot::Banner, crate::banner::console_context(true));
        assert_eq!(
            full,
            format!(
                "{}\n",
                crate::banner::line(&aspect_telemetry::cargo_pkg_short_version())
            )
        );
    }

    #[test]
    fn plain_is_full_without_emojis() {
        let phase = minijinja::context! {
            title => "Build", emoji => "🔨", highlight_style => "<s>", highlight_reset => "<r>", text => "Building"
        };
        let c = console("plain", &Overrides::default(), false);
        assert_eq!(
            c.render(Slot::Header, header()),
            "→ <c>Running<r> test task"
        );
        assert_eq!(
            c.render(Slot::Phase, phase.clone()),
            "\n→ <s>Build<r> · Building"
        );
        assert_eq!(c.render(Slot::Progress, text("12 of 40")), "→ 12 of 40");
        assert_eq!(
            c.render(Slot::Verdict, verdict(true)),
            "\n→ Failed test task (exit code 3) in 1.2s · 4 tests — Build 1.0s"
        );
        let bk = console("plain", &Overrides::default(), true);
        assert_eq!(bk.render(Slot::Header, header()), "<c>Running<r> test task");
        assert_eq!(bk.render(Slot::Phase, phase), "<s>Build<r> · Building");
        assert_eq!(
            bk.render(Slot::Verdict, verdict(false)),
            "Passed · test task in 1.2s · 4 tests — Build 1.0s"
        );
    }

    #[test]
    fn compact_keeps_only_the_verdict_without_its_breakdown() {
        let c = console("compact", &Overrides::default(), false);
        for slot in [Slot::Banner, Slot::Header, Slot::Phase, Slot::Progress] {
            assert_eq!(c.render(slot, header()), "", "{slot:?}");
        }
        assert_eq!(
            c.render(Slot::Verdict, verdict(true)),
            "→ ❌ Failed test task (exit code 3) in 1.2s · 4 tests"
        );
        let bk = console("compact", &Overrides::default(), true);
        assert_eq!(
            bk.render(Slot::Verdict, verdict(false)),
            ":white_check_mark: Passed · test task in 1.2s · 4 tests"
        );
    }

    #[test]
    fn buildkite_sections_never_render_blank() {
        let mut blank_header = Overrides::default();
        blank_header.set("header", "{# blank #}".into()).unwrap();
        for (preset, overrides) in [("silent", Overrides::default()), ("full", blank_header)] {
            let c = console(preset, &overrides, true);
            assert_eq!(
                c.render(Slot::Header, header()),
                ":aspect: <c>Running<r> test task"
            );
        }
        let silent = console("silent", &Overrides::default(), true);
        assert_eq!(
            silent.render(Slot::Progress, text("x")),
            "",
            "progress opens no section"
        );
        let off_bk = console("silent", &Overrides::default(), false);
        assert_eq!(off_bk.render(Slot::Verdict, verdict(true)), "");
    }

    #[test]
    fn a_blank_buildkite_section_keeps_the_presets_text() {
        let mut blank_header = Overrides::default();
        blank_header.set("header", "".into()).unwrap();
        let c = console("plain", &blank_header, true);
        assert_eq!(c.render(Slot::Header, header()), "<c>Running<r> test task");
    }

    #[test]
    fn a_buildkite_section_drops_leading_whitespace() {
        let phase = minijinja::context! { title => "Build", text => "Building" };
        for template in [
            "\n→ {{ title }} · {{ text }}",
            "  \n\t→ {{ title }} · {{ text }}",
        ] {
            let mut o = Overrides::default();
            o.set("phase", template.into()).unwrap();
            assert_eq!(
                console("full", &o, true).render(Slot::Phase, phase.clone()),
                "→ Build · Building",
                "{template:?}"
            );
        }
        let mut o = Overrides::default();
        o.set("phase", "\n→ {{ title }} · {{ text }}".into())
            .unwrap();
        assert_eq!(
            console("full", &o, false).render(Slot::Phase, phase),
            "\n→ Build · Building",
            "off Buildkite the line keeps its spacing"
        );
    }

    #[test]
    fn overrides_layer_flat_then_task_kind() {
        let mut o = Overrides::default();
        o.set("progress", "flat {{ text }}".into()).unwrap();
        o.set("header", "flat {{ task.path }}".into()).unwrap();
        o.set_for_task("test", "progress", "test {{ text }}".into())
            .unwrap();
        o.set_for_task("lint", "header", "lint".into()).unwrap();
        let c = console("silent", &o, false);
        assert_eq!(c.render(Slot::Progress, text("x")), "test x");
        assert_eq!(c.render(Slot::Header, header()), "flat test");
        assert_eq!(
            c.render(Slot::Verdict, verdict(true)),
            "",
            "the preset fills the rest"
        );
    }

    fn grouped(kind: &str, path: &str) -> TaskIdentity {
        TaskIdentity {
            kind: kind.into(),
            name: kind.into(),
            path: path.into(),
            ..Default::default()
        }
    }

    fn resolved(preset: &str, o: &Overrides, task: TaskIdentity) -> Console {
        Console::resolve(preset, o, task, Host::default()).unwrap()
    }

    #[test]
    fn a_command_path_entry_beats_its_kind_entry() {
        let mut o = Overrides::default();
        o.set_for_task("add", "progress", "kind".into()).unwrap();
        o.set_for_task("worktree add", "progress", "path".into())
            .unwrap();
        let progress = |task| resolved("full", &o, task).render(Slot::Progress, text("x"));
        assert_eq!(progress(grouped("add", "worktree add")), "path");
        assert_eq!(progress(grouped("add", "auth add")), "kind");
        assert_eq!(progress(task()), "→ x");
    }

    #[test]
    fn a_task_can_pick_its_own_preset() {
        let mut o = Overrides::default();
        o.set_preset_for_task("add", "silent".into()).unwrap();
        o.set_preset_for_task("worktree add", "compact".into())
            .unwrap();
        o.set("progress", "everyone".into()).unwrap();
        o.set_for_task("add", "header", "mine".into()).unwrap();

        let c = resolved("full", &o, grouped("add", "worktree add"));
        assert_eq!(
            c.preset(),
            "compact",
            "the command path's preset beats the kind's"
        );
        assert_eq!(
            c.render(Slot::Progress, text("x")),
            "",
            "the task's preset leaves out templates"
        );
        assert_eq!(
            c.render(Slot::Header, header()),
            "mine",
            "task_templates beat the task's preset"
        );

        assert_eq!(
            resolved("full", &o, grouped("add", "auth add")).preset(),
            "silent"
        );
        let other = resolved("full", &o, task());
        assert_eq!(other.preset(), "full");
        assert_eq!(other.render(Slot::Progress, text("x")), "everyone");
    }

    #[test]
    fn a_command_line_preset_beats_task_presets() {
        let mut o = Overrides::default();
        o.set_preset_for_task("check", "silent".into()).unwrap();
        o.set("progress", "everyone".into()).unwrap();
        o.preset_from_cli();
        let c = resolved("full", &o, task());
        assert_eq!(c.preset(), "full");
        assert_eq!(c.render(Slot::Progress, text("x")), "everyone");
    }

    #[test]
    fn a_slot_name_is_no_task_key() {
        let err = Overrides::default()
            .set_preset_for_task("verdict", "silent".into())
            .unwrap_err();
        assert!(
            err.contains(r#"task_presets["verdict"] names a slot"#),
            "{err}"
        );
        let err = Overrides::default()
            .set_for_task("header", "verdict", "x".into())
            .unwrap_err();
        assert!(
            err.contains(r#"task_templates["header"] names a slot"#),
            "{err}"
        );
    }

    #[test]
    fn an_unknown_task_preset_is_refused_for_every_task() {
        let mut o = Overrides::default();
        o.set_preset_for_task("lint", "loud".into()).unwrap();
        let err = Console::resolve("full", &o, task(), Host::default())
            .map(|_| ())
            .unwrap_err();
        assert!(
            err.contains(r#"task_presets["lint"]: unknown template preset "loud""#),
            "{err}"
        );
    }

    #[test]
    fn a_failing_override_falls_back_to_its_preset() {
        let mut o = Overrides::default();
        o.set("progress", "{{ text.nope.deeper }}".into()).unwrap();
        assert_eq!(
            console("full", &o, false).render(Slot::Progress, text("x")),
            "→ x"
        );
        assert_eq!(
            console("silent", &o, false).render(Slot::Progress, text("x")),
            ""
        );
    }

    #[test]
    fn task_identity_reaches_every_slot() {
        let mut o = Overrides::default();
        o.set(
            "verdict",
            "{{ task.kind }}|{{ task.path }}|{{ buildkite }}".into(),
        )
        .unwrap();
        assert_eq!(
            console("full", &o, false).render(Slot::Verdict, verdict(false)),
            "test|test|false"
        );
    }

    #[test]
    fn resolve_refuses_what_could_never_render() {
        let resolve = |preset: &str, o: &Overrides| {
            Console::resolve(preset, o, task(), Host::default())
                .map(|_| ())
                .unwrap_err()
        };
        assert!(
            resolve("loud", &Overrides::default()).contains("unknown template preset \"loud\"")
        );

        let mut o = Overrides::default();
        o.set_for_task("other", "verdict", "{% if %}".into())
            .unwrap();
        assert!(
            resolve("full", &o).contains(r#"task_templates["other"]["verdict"] does not compile"#),
            "a block for another task is still checked"
        );

        let mut o = Overrides::default();
        o.set_for_task("test", "verdict", "{% if %}".into())
            .unwrap();
        assert!(
            resolve("full", &o).contains(r#"task_templates["test"]["verdict"] does not compile"#)
        );

        let mut o = Overrides::default();
        o.set("phase", "{{ unclosed".into()).unwrap();
        assert!(resolve("full", &o).contains(r#"templates["phase"] does not compile"#));

        let err = Overrides::default().set("verdcit", "x".into()).unwrap_err();
        assert!(
            err.contains(r#"templates: "verdcit" is not a slot"#),
            "{err}"
        );
        let err = Overrides::default()
            .set_for_task("lint", "verdcit", "x".into())
            .unwrap_err();
        assert!(
            err.contains(r#"task_templates["lint"]: "verdcit" is not a slot"#),
            "{err}"
        );
    }

    #[test]
    fn every_preset_template_compiles_and_the_default_is_listed() {
        let env = Environment::new();
        for p in PRESETS {
            for slot in Slot::ALL {
                env.template_from_str(p.template(slot))
                    .unwrap_or_else(|e| panic!("{} {slot:?}: {e}", p.name));
            }
        }
        assert!(PRESETS.iter().any(|p| p.name == DEFAULT_PRESET));
    }

    /// The command paths of a repo with built-in, grouped, alias and custom
    /// tasks. `buildkite_annotations_test.axl` checks the CI surfaces against the
    /// same table.
    fn task_paths() -> Vec<String> {
        [
            "build",
            "test",
            "lint",
            "format",
            "buildifier",
            "deploy",
            "worktree add",
            "worktree release",
        ]
        .map(String::from)
        .to_vec()
    }

    /// The cases `text_test.axl` checks AXL's `edit_distance` on.
    #[test]
    fn edit_distance_counts_characters() {
        for (a, b, expected) in [
            ("test", "test", 0),
            ("lnt", "lint", 1),
            ("tset", "test", 2),
            ("", "add", 3),
            ("tést", "test", 1),
        ] {
            assert_eq!(edit_distance(a, b), expected, "{a} → {b}");
        }
    }

    #[test]
    fn a_near_miss_names_the_closest_task() {
        for (key, expected) in [
            ("tset", Some("test")),
            ("bulid", Some("build")),
            ("lnt", Some("lint")),
            ("worktree ad", Some("worktree add")),
            ("relase", Some("release")),
            ("buildifer", Some("buildifier")),
            ("deplyo", Some("deploy")),
            ("test", None),
            ("add", None),
            ("worktree add", None),
            ("release-notes", None),
            ("fmt", None),
            ("abc", None),
            ("ad", None),
            ("tt", None),
        ] {
            assert_eq!(
                closest_task_key(key, &task_paths()).as_deref(),
                expected,
                "{key}"
            );
        }
    }

    #[test]
    fn a_near_miss_is_warned_about_once_per_key() {
        let mut o = Overrides::default();
        o.set_preset_for_task("tset", "silent".into()).unwrap();
        o.set_for_task("tset", "header", "a".into()).unwrap();
        o.set_for_task("tset", "verdict", "b".into()).unwrap();
        o.set_for_task("worktree ad", "header", "c".into()).unwrap();
        o.set_for_task("release-notes", "header", "d".into())
            .unwrap();
        o.set_preset_for_task("test", "plain".into()).unwrap();
        assert_eq!(
            o.near_miss_warnings(&task_paths()),
            [
                r#"Console task_presets["tset"] and task_templates["tset"] name no task here; did you mean "test"?"#,
                r#"Console task_templates["worktree ad"] names no task here; did you mean "worktree add"?"#,
            ]
        );
        assert!(o.near_miss_warnings(&[]).is_empty());
    }
}
