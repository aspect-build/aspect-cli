//! Single-file bridge between the runtime (TaskLike / FeatureLike) and Clap.
//!
//! Public surface:
//!
//! * [`Cmd`] — value-passing input. Holds task & feature trait objects plus
//!   filesystem context. `build()` produces the Clap [`Command`]. `dispatch()`
//!   turns parsed [`ArgMatches`] into a [`Dispatch`].
//! * [`Dispatch`] — carries `task_id`, `task_key`, `task_uuid`, and the parsed
//!   matches. `task_args(...)` and `feature_args(...)` produce the merged
//!   runtime [`Arguments`] for a given task or feature.
//!
//! Everything else in this module is private.

use std::collections::BTreeMap;
use std::path::Path;

use axl_runtime::engine::arg::Arg;
use axl_runtime::engine::arguments::Arguments;
use axl_runtime::engine::feature::FeatureLike;
use axl_runtime::engine::names::to_display_name;
use axl_runtime::engine::task::{MAX_TASK_GROUPS, TaskLike};
use axl_runtime::eval::TimingMode;
use axl_runtime::module::Mod;
use clap::builder::{PossibleValuesParser, Resettable, StyledStr};
use clap::parser::ValueSource;
use clap::{Arg as ClapArg, ArgAction, ArgMatches, Command, value_parser};
use starlark::collections::SmallMap;
use starlark::values::list::{AllocList, ListRef};
use starlark::values::none::NoneType;
use starlark::values::{Heap, Value, ValueLike};
use thiserror::Error;

const TASK_ID_KEY: &str = "@@@$'__AXL_TASK_ID__'$@@@";
const TASK_COMMAND_DISPLAY_ORDER: usize = 0;
const TASK_GROUP_DISPLAY_ORDER: usize = 1;

#[derive(Error, Debug)]
pub enum CmdError {
    #[error("task {0:?} (defined in {1:?}) cannot have more than {2} group levels")]
    TooManyGroups(String, String, usize),
    #[error("task {0:?} in group {1:?} (defined in {2:?}) conflicts with another task or group")]
    NameConflict(String, Vec<String>, String),
    #[error("no task selected")]
    NoTaskSelected,
}

/// Bridge from runtime values (tasks, features, modules) to a parsed Clap surface.
pub struct Cmd<'a, 'v> {
    pub tasks: Vec<&'v dyn TaskLike<'v>>,
    pub features: Vec<&'v dyn FeatureLike<'v>>,
    pub repo_root: &'a Path,
    pub modules: &'a [Mod],
}

/// Carries the parsed CLI state needed to execute the selected task and its features.
pub struct Dispatch {
    pub task_id: usize,
    pub task_key: String,
    pub task_uuid: Option<String>,
    pub timing: TimingMode,
    matches: ArgMatches,
}

impl<'a, 'v> Cmd<'a, 'v> {
    /// Build the full root Clap command (`aspect`) with all task subcommands.
    pub fn build(&self, version: &str) -> Result<Command, CmdError> {
        let feature_blocks: Vec<FeatureBlock> = self
            .features
            .iter()
            .filter_map(|f| feature_block(*f, self.repo_root, self.modules))
            .collect();

        let mut tree = Tree::default();
        for (idx, task) in self.tasks.iter().enumerate() {
            let label = defined_in_label(task.path(), self.repo_root, self.modules);
            let task_cmd = task_command(idx, *task, &label, &feature_blocks);
            tree.insert(*task, &label, task_cmd)?;
        }

        let group_section = group_section(&tree.group_names());

        let root = Command::new("aspect")
            .bin_name("aspect")
            .about("Aspect's programmable task runner built on top of Bazel\n{ Correct, Fast, Usable } -- Choose three")
            .subcommand_value_name("TASK|GROUP|COMMAND")
            .disable_help_subcommand(true)
            .version(version.to_owned())
            .disable_version_flag(true)
            .arg(
                ClapArg::new("version")
                    .short('v')
                    .long("version")
                    .action(ArgAction::Version)
                    .help("Print version"),
            )
            .arg(
                ClapArg::new("task-key")
                    .long("task-key")
                    .value_name("KEY")
                    .global(true)
                    .value_parser(parse_task_key)
                    .help("A short key identifying this task invocation. Allowed characters: A-Za-z0-9, _, -. Useful when the same task runs multiple times in one pipeline (e.g. 'backend', 'frontend'). Auto-generated if not set."),
            )
            .arg(
                ClapArg::new("task-id")
                    .long("task-id")
                    .value_name("UUID")
                    .global(true)
                    .value_parser(parse_task_uuid)
                    .help("A UUID uniquely identifying this task invocation. Auto-generated if not set."),
            )
            .arg(
                ClapArg::new("timing")
                    .long("timing")
                    .value_name("LEVEL")
                    .global(true)
                    .value_parser(parse_timing_mode)
                    .default_value("detailed")
                    .help("Verbosity of the phase-timing breakdown trailing the task completion line: 'total' (no breakdown — total only), 'summary' (inline phases), or 'detailed' (multi-line with descriptions; default). Tasks that don't opt into phases see only the total regardless of this setting."),
            )
            .subcommand(Command::new("version").about("Print version").hide(true))
            .subcommand(
                Command::new("help")
                    .about("Print this message or the help of the given subcommand(s)")
                    .hide(true),
            )
            .help_template(format!(
                "{{about-with-newline}}\n{{usage-heading}} {{usage}}\n\n\x1b[1;4mTasks:\x1b[0m\n{{subcommands}}{group_section}\n\x1b[1;4mCommands:\x1b[0m\n  \x1b[1mversion\x1b[0m  Print version\n  \x1b[1mhelp\x1b[0m     Print this message or the help of the given subcommand(s)\n\n\x1b[1;4mOptions:\x1b[0m\n{{options}}"
            ));

        tree.attach(root)
    }

    /// Walk the parsed matches to the deepest subcommand and extract task ids + uuids.
    pub fn dispatch(&self, matches: ArgMatches) -> Result<Dispatch, CmdError> {
        let leaf = deepest_subcommand(&matches).ok_or(CmdError::NoTaskSelected)?;
        let task_id = *leaf
            .get_one::<usize>(TASK_ID_KEY)
            .ok_or(CmdError::NoTaskSelected)?;
        let task_key = leaf
            .get_one::<String>("task-key")
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(generate_task_key);
        let task_uuid = leaf.get_one::<String>("task-id").cloned();
        let timing = leaf
            .get_one::<TimingMode>("timing")
            .copied()
            .unwrap_or_default();
        Ok(Dispatch {
            task_id,
            task_key,
            task_uuid,
            timing,
            matches,
        })
    }
}

impl Dispatch {
    /// Build the merged `Arguments` for the selected task.
    pub fn task_args<'v>(&self, task: &dyn TaskLike<'v>, heap: Heap<'v>) -> Arguments<'v> {
        let leaf = deepest_subcommand(&self.matches).expect("dispatch built from valid matches");
        merge_args(task.args(), task.overrides(), leaf, heap, Scope::Task)
    }

    /// Build the merged `Arguments` for a feature implementation invocation.
    pub fn feature_args<'v>(&self, feat: &dyn FeatureLike<'v>, heap: Heap<'v>) -> Arguments<'v> {
        let leaf = deepest_subcommand(&self.matches).expect("dispatch built from valid matches");
        let prefix = feat.name();
        merge_args(
            feat.args(),
            feat.overrides(),
            leaf,
            heap,
            Scope::Feature(&prefix),
        )
    }
}

// ── Scope: how an arg's Clap key is derived ────────────────────────────────

#[derive(Copy, Clone)]
enum Scope<'a> {
    Task,
    Feature(&'a str),
}

fn clap_id(scope: Scope<'_>, name: &str, arg: &Arg) -> String {
    match scope {
        Scope::Task => name.to_owned(),
        Scope::Feature(prefix) => {
            if let Some(lo) = arg.long_override() {
                return lo.to_owned();
            }
            let long = name.replace('_', "-");
            if prefix.is_empty() {
                long
            } else {
                format!("{}:{}", prefix, long)
            }
        }
    }
}

fn long_flag(scope: Scope<'_>, name: &str, arg: &Arg) -> String {
    if let Some(lo) = arg.long_override() {
        return lo.to_owned();
    }
    match scope {
        Scope::Task => name.replace('_', "-"),
        Scope::Feature(prefix) => {
            let long = name.replace('_', "-");
            if prefix.is_empty() {
                long
            } else {
                format!("{}:{}", prefix, long)
            }
        }
    }
}

// ── Arg → ClapArg ──────────────────────────────────────────────────────────

fn arg_to_clap(scope: Scope<'_>, name: &str, arg: &Arg) -> ClapArg {
    let id = clap_id(scope, name, arg);
    let long = long_flag(scope, name, arg);
    match arg {
        Arg::String {
            required,
            default,
            short,
            values,
            description,
            ..
        } => {
            let mut it = ClapArg::new(id.clone())
                .long(long.clone())
                .value_name(long)
                .short(short_char(short))
                .help(help_text(description))
                .required(*required)
                .default_value(default.clone());
            it = if let Some(values) = values {
                it.value_parser(PossibleValuesParser::new(values))
            } else {
                it.value_parser(value_parser!(String))
            };
            it
        }
        Arg::Boolean {
            required,
            default,
            short,
            description,
            ..
        } => ClapArg::new(id)
            .long(long.clone())
            .value_name(long)
            .short(short_char(short))
            .help(help_text(description))
            .required(*required)
            .default_value(if *default { "true" } else { "false" })
            .value_parser(value_parser!(bool))
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value("true"),
        Arg::Int {
            required,
            default,
            short,
            description,
            ..
        } => ClapArg::new(id)
            .long(long.clone())
            .value_name(long)
            .short(short_char(short))
            .help(help_text(description))
            .required(*required)
            .default_value(default.to_string())
            .value_parser(value_parser!(i32)),
        Arg::UInt {
            required,
            default,
            short,
            description,
            ..
        } => ClapArg::new(id)
            .long(long.clone())
            .value_name(long)
            .short(short_char(short))
            .help(help_text(description))
            .required(*required)
            .default_value(default.to_string())
            .value_parser(value_parser!(u32)),
        Arg::Positional {
            minimum,
            maximum,
            default,
            description,
        } => {
            let mut it = ClapArg::new(id.clone())
                .value_parser(value_parser!(String))
                .value_name(id)
                .help(help_text(description))
                .num_args(*minimum as usize..=*maximum as usize);
            if let Some(default) = default {
                it = it.default_values(default);
            }
            it
        }
        Arg::TrailingVarArgs { description } => ClapArg::new(id.clone())
            .value_parser(value_parser!(String))
            .value_name(id)
            .help(help_text(description))
            .allow_hyphen_values(true)
            .last(true)
            .num_args(0..),
        Arg::StringList {
            required,
            default,
            short,
            description,
            ..
        } => {
            let mut it = ClapArg::new(id)
                .long(long.clone())
                .value_name(long)
                .short(short_char(short))
                .help(help_text(description))
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(String));
            if !default.is_empty() {
                it = it.default_values(default.iter().cloned());
            }
            it
        }
        Arg::BooleanList {
            required,
            default,
            short,
            description,
            ..
        } => {
            let mut it = ClapArg::new(id)
                .long(long.clone())
                .value_name(long)
                .short(short_char(short))
                .help(help_text(description))
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(bool))
                .num_args(0..=1)
                .default_missing_value("true");
            if !default.is_empty() {
                it = it.default_values(default.iter().map(|&b| if b { "true" } else { "false" }));
            }
            it
        }
        Arg::IntList {
            required,
            default,
            short,
            description,
            ..
        } => {
            let mut it = ClapArg::new(id)
                .long(long.clone())
                .value_name(long)
                .short(short_char(short))
                .help(help_text(description))
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(i32));
            if !default.is_empty() {
                it = it.default_values(default.iter().map(|i| i.to_string()));
            }
            it
        }
        Arg::UIntList {
            required,
            default,
            short,
            description,
            ..
        } => {
            let mut it = ClapArg::new(id)
                .long(long.clone())
                .value_name(long)
                .short(short_char(short))
                .help(help_text(description))
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(u32));
            if !default.is_empty() {
                it = it.default_values(default.iter().map(|u| u.to_string()));
            }
            it
        }
        Arg::Custom { .. } => unreachable!("Custom args are not CLI-exposed"),
    }
}

// Apply a config.axl override to the just-built ClapArg: sets the default and
// drops `required` (the user has supplied the value, just not on the CLI).
fn with_override(mut clap_arg: ClapArg, arg: &Arg, override_strings: &[String]) -> ClapArg {
    clap_arg = match arg {
        Arg::StringList { .. }
        | Arg::BooleanList { .. }
        | Arg::IntList { .. }
        | Arg::UIntList { .. }
        | Arg::TrailingVarArgs { .. } => clap_arg.default_values(override_strings.iter().cloned()),
        _ => {
            if let Some(v) = override_strings.first() {
                clap_arg.default_value(v.clone())
            } else {
                clap_arg
            }
        }
    };
    clap_arg.required(false)
}

// ── Match → Arguments merge ────────────────────────────────────────────────

fn merge_args<'v>(
    schema: &SmallMap<String, Arg>,
    overrides_value: Value<'v>,
    matches: &ArgMatches,
    heap: Heap<'v>,
    scope: Scope<'_>,
) -> Arguments<'v> {
    let args = Arguments::new();
    for (name, arg) in schema.iter() {
        match arg {
            Arg::Custom { default, .. } => {
                let v = default
                    .map(|fv| fv.to_value())
                    .unwrap_or_else(|| heap.alloc(NoneType));
                args.insert(name.clone(), v);
            }
            cli => {
                let key = clap_id(scope, name, cli);
                if let Some(val) = clap_to_value(cli, &key, matches, heap) {
                    args.insert(name.clone(), val);
                }
            }
        }
    }

    // Layer config.axl overrides for keys the user did NOT pass on the CLI.
    let Some(overrides) = overrides_value.downcast_ref::<Arguments>() else {
        return args;
    };
    for (k, v) in overrides.entries() {
        let user_explicit = schema
            .get(&k)
            .filter(|a| a.is_cli_exposed())
            .map(|a| matches.value_source(&clap_id(scope, &k, a)) == Some(ValueSource::CommandLine))
            .unwrap_or(false);
        if !user_explicit {
            args.insert(k, v);
        }
    }
    args
}

fn clap_to_value<'v>(
    arg: &Arg,
    key: &str,
    matches: &ArgMatches,
    heap: Heap<'v>,
) -> Option<Value<'v>> {
    let v = match arg {
        Arg::Custom { .. } => return None,
        Arg::String { .. } => heap
            .alloc_str(matches.get_one::<String>(key).unwrap_or(&String::new()))
            .to_value(),
        Arg::Int { .. } => heap
            .alloc(*matches.get_one::<i32>(key).unwrap_or(&0))
            .to_value(),
        Arg::UInt { .. } => heap
            .alloc(*matches.get_one::<u32>(key).unwrap_or(&0))
            .to_value(),
        Arg::Boolean { .. } => heap.alloc(*matches.get_one::<bool>(key).unwrap_or(&false)),
        Arg::Positional { .. } | Arg::TrailingVarArgs { .. } => heap.alloc(AllocList(
            matches
                .get_many::<String>(key)
                .map_or(vec![], |it| it.map(|s| s.as_str()).collect()),
        )),
        Arg::StringList { .. } => heap.alloc(AllocList(
            matches
                .get_many::<String>(key)
                .unwrap_or_default()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
        )),
        Arg::BooleanList { .. } => heap.alloc(AllocList(
            matches
                .get_many::<bool>(key)
                .unwrap_or_default()
                .copied()
                .collect::<Vec<_>>(),
        )),
        Arg::IntList { .. } => heap.alloc(AllocList(
            matches
                .get_many::<i32>(key)
                .unwrap_or_default()
                .copied()
                .collect::<Vec<_>>(),
        )),
        Arg::UIntList { .. } => heap.alloc(AllocList(
            matches
                .get_many::<u32>(key)
                .unwrap_or_default()
                .copied()
                .collect::<Vec<_>>(),
        )),
    };
    Some(v)
}

// ── Override stringification (for Clap default_value) ──────────────────────

fn stringify_overrides(
    overrides_value: Value<'_>,
) -> std::collections::HashMap<String, Vec<String>> {
    let Some(args) = overrides_value.downcast_ref::<Arguments>() else {
        return std::collections::HashMap::new();
    };
    args.entries()
        .into_iter()
        .map(|(k, v)| {
            let elements = if let Some(list) = ListRef::from_value(v) {
                list.iter().map(stringify_value).collect()
            } else {
                vec![stringify_value(v)]
            };
            (k, elements)
        })
        .collect()
}

fn stringify_value(v: Value<'_>) -> String {
    if let Some(s) = v.unpack_str() {
        return s.to_owned();
    }
    let s = v.to_string();
    if v.get_type() == "bool" {
        s.to_lowercase()
    } else {
        s
    }
}

// ── Per-feature help block (built once, applied to every task command) ─────

struct FeatureBlock {
    args: Vec<(String, Arg)>,
    heading: String,
    description_line: String,
    prefix: String,
}

fn feature_block(
    feat: &dyn FeatureLike<'_>,
    repo_root: &Path,
    modules: &[Mod],
) -> Option<FeatureBlock> {
    let mut args: Vec<(String, Arg)> = feat
        .args()
        .iter()
        .filter(|(_, v)| v.is_cli_exposed())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if args.is_empty() {
        return None;
    }

    // Bake config.axl overrides into the schema defaults so `--help` shows them.
    for (k, vals) in stringify_overrides(feat.overrides()) {
        let Some((_, arg)) = args.iter_mut().find(|(name, _)| name == &k) else {
            continue;
        };
        let Some(first) = vals.first() else { continue };
        match arg {
            Arg::Boolean { default, .. } => *default = first == "true",
            Arg::String { default, .. } => *default = first.clone(),
            Arg::Int { default, .. } => {
                if let Ok(v) = first.parse() {
                    *default = v;
                }
            }
            Arg::UInt { default, .. } => {
                if let Ok(v) = first.parse() {
                    *default = v;
                }
            }
            _ => {}
        }
    }

    let identifier = feat.export_name().unwrap_or_default();
    let label = defined_in_label(feat.path(), repo_root, modules);
    let heading = format!("{} Options", feat.display_name());
    let context = format!(
        "\x1b[3m{}\x1b[0m feature defined in \x1b[3m{}\x1b[0m",
        identifier, label
    );
    let body = if !feat.description().is_empty() {
        feat.description().clone()
    } else if !feat.summary().is_empty() {
        feat.summary().clone()
    } else {
        String::new()
    };
    let desc_text = if body.is_empty() {
        context
    } else {
        format!("{}\n\n      {}", body, context)
    };
    let description_line = format!("\x1b[0m      {}\n\x1b[8m", desc_text);
    Some(FeatureBlock {
        args,
        heading,
        description_line,
        prefix: feat.name(),
    })
}

// ── Per-task subcommand assembly ───────────────────────────────────────────

fn task_command(
    index: usize,
    task: &dyn TaskLike<'_>,
    label: &str,
    feature_blocks: &[FeatureBlock],
) -> Command {
    let name = task.name();
    let display_name = task.display_name();
    let display = if !display_name.is_empty() {
        display_name
    } else {
        to_display_name(&name)
    };

    let context_line = format!(
        "\x1b[3m{}\x1b[0m task defined in \x1b[3m{}\x1b[0m",
        name, label,
    );
    let about = if task.summary().is_empty() {
        context_line.clone()
    } else {
        task.summary().clone()
    };
    let help_header = if task.summary().is_empty() && task.description().is_empty() {
        None
    } else {
        let body = if !task.description().is_empty() {
            task.description().clone()
        } else {
            task.summary().clone()
        };
        Some(format!("{}\n\n{}", body, context_line))
    };

    let mut cmd = Command::new(name)
        .about(about)
        .display_order(TASK_COMMAND_DISPLAY_ORDER)
        .arg(
            ClapArg::new(TASK_ID_KEY)
                .long(TASK_ID_KEY)
                .hide(true)
                .hide_default_value(true)
                .hide_short_help(true)
                .hide_possible_values(true)
                .hide_long_help(true)
                .value_parser(value_parser!(usize))
                .default_value(index.to_string()),
        );

    let overrides = stringify_overrides(task.overrides());
    let heading = format!("{} Options", display);
    for (arg_name, arg) in task.cli_args() {
        let mut clap_arg = arg_to_clap(Scope::Task, arg_name, arg).help_heading(heading.clone());
        if let Some(over) = overrides.get(arg_name) {
            clap_arg = with_override(clap_arg, arg, over);
        }
        cmd = cmd.arg(clap_arg);
    }

    for block in feature_blocks {
        let full_heading = if block.description_line.is_empty() {
            block.heading.clone()
        } else {
            format!("{}:\n{}", block.heading, block.description_line)
        };
        for (arg_name, arg) in block.args.iter() {
            let clap_arg = arg_to_clap(Scope::Feature(&block.prefix), arg_name, arg)
                .help_heading(full_heading.clone());
            cmd = cmd.arg(clap_arg);
        }
    }

    if let Some(header) = help_header {
        cmd = cmd.help_template(format!(
            "{header}\n\n{{usage-heading}} {{usage}}\n\n{{all-args}}\n"
        ));
    }
    cmd
}

// ── Subgroup tree ──────────────────────────────────────────────────────────

#[derive(Default)]
struct Tree {
    subgroups: BTreeMap<String, Tree>,
    tasks: BTreeMap<String, Command>,
}

impl Tree {
    fn insert(
        &mut self,
        task: &dyn TaskLike<'_>,
        label: &str,
        cmd: Command,
    ) -> Result<(), CmdError> {
        let group = task.group().clone();
        if group.len() > MAX_TASK_GROUPS {
            return Err(CmdError::TooManyGroups(
                task.name(),
                label.to_owned(),
                MAX_TASK_GROUPS,
            ));
        }
        self.insert_at(&task.name(), &group[..], &group[..], label, cmd)
    }

    fn insert_at(
        &mut self,
        name: &str,
        full_group: &[String],
        remaining: &[String],
        label: &str,
        cmd: Command,
    ) -> Result<(), CmdError> {
        if remaining.is_empty() {
            if self.subgroups.contains_key(name) || self.tasks.contains_key(name) {
                return Err(CmdError::NameConflict(
                    name.to_owned(),
                    full_group.to_vec(),
                    label.to_owned(),
                ));
            }
            self.tasks.insert(name.to_owned(), cmd);
            return Ok(());
        }
        let head = &remaining[0];
        if self.tasks.contains_key(head) {
            return Err(CmdError::NameConflict(
                head.clone(),
                full_group.to_vec(),
                label.to_owned(),
            ));
        }
        let sub = self.subgroups.entry(head.clone()).or_default();
        sub.insert_at(name, full_group, &remaining[1..], label, cmd)
    }

    fn group_names(&self) -> Vec<String> {
        self.subgroups.keys().cloned().collect()
    }

    fn attach(self, mut root: Command) -> Result<Command, CmdError> {
        // Subgroups (rendered hidden, with their own help template).
        for (name, sub) in self.subgroups {
            let sub_groups = sub.group_names();
            let mut template = String::from("{about-with-newline}\n{usage-heading} {usage}");
            if !sub.tasks.is_empty() {
                template.push_str("\n\n\x1b[1;4mTasks:\x1b[0m\n{subcommands}");
            }
            if !sub_groups.is_empty() {
                let max_len = sub_groups.iter().map(|n| n.len()).max().unwrap_or(0);
                template.push_str("\n\n\x1b[1;4mTask Groups:\x1b[0m\n");
                for gname in &sub_groups {
                    let pad = " ".repeat(max_len - gname.len() + 2);
                    template.push_str(&format!("  \x1b[1m{}\x1b[0m{}task group\n", gname, pad));
                }
            }
            let template = format!(
                "{}\n\n\x1b[1;4mOptions:\x1b[0m\n{{options}}",
                template.trim_end()
            );
            let mut subcmd = Command::new(name.clone())
                .subcommand_value_name("TASK|GROUP")
                .about(format!("\x1b[3m{}\x1b[0m task group", name))
                .display_order(TASK_GROUP_DISPLAY_ORDER)
                .hide(true)
                .help_template(template);
            subcmd = sub.attach(subcmd)?;
            if root.find_subcommand(&name).is_some() {
                return Err(CmdError::NameConflict(name, vec![], String::new()));
            }
            root = root.subcommand(subcmd);
        }
        // Leaf task subcommands.
        for (name, subcmd) in self.tasks {
            if root.find_subcommand(&name).is_some() {
                return Err(CmdError::NameConflict(name, vec![], String::new()));
            }
            root = root.subcommand(subcmd);
        }
        if root.get_subcommands().next().is_some() {
            root = root.arg_required_else_help(true);
        }
        Ok(root)
    }
}

fn group_section(group_names: &[String]) -> String {
    if group_names.is_empty() {
        return String::new();
    }
    let max_len = group_names.iter().map(|n| n.len()).max().unwrap_or(0);
    let mut s = String::from("\n\n\x1b[1;4mTask Groups:\x1b[0m\n");
    for name in group_names {
        let pad = " ".repeat(max_len - name.len() + 2);
        s.push_str(&format!(
            "  \x1b[1m{}\x1b[0m{}\x1b[3m{}\x1b[0m task group\n",
            name, pad, name
        ));
    }
    s
}

// ── Misc helpers ───────────────────────────────────────────────────────────

fn defined_in_label(path: &Path, repo_root: &Path, modules: &[Mod]) -> String {
    for r#mod in modules {
        if r#mod.is_root() || !path.starts_with(&r#mod.root) {
            continue;
        }
        let rel = path.strip_prefix(&r#mod.root).unwrap_or(path);
        return format!("@{}//{}", r#mod.name, rel.display());
    }
    let rel = path.strip_prefix(repo_root).unwrap_or(path);
    format!("{}", rel.display())
}

fn deepest_subcommand(matches: &ArgMatches) -> Option<&ArgMatches> {
    let (_, mut leaf) = matches.subcommand()?;
    while let Some((_, next)) = leaf.subcommand() {
        leaf = next;
    }
    Some(leaf)
}

fn parse_task_key(s: &str) -> Result<String, String> {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Ok(s.to_owned())
    } else {
        Err(format!(
            "'{}' contains invalid characters (allowed: A-Za-z0-9, _, -)",
            s
        ))
    }
}

fn parse_task_uuid(s: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(s)
        .map(|u| u.to_string())
        .map_err(|_| {
            format!(
                "'{}' is not a valid UUID (expected xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)",
                s
            )
        })
}

fn parse_timing_mode(s: &str) -> Result<TimingMode, String> {
    s.parse::<TimingMode>()
}

fn generate_task_key() -> String {
    names::Generator::with_naming(names::Name::Plain)
        .next()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()[..8].to_string())
}

fn help_text(description: &Option<String>) -> Resettable<StyledStr> {
    match description {
        Some(text) => Resettable::Value(text.into()),
        None => Resettable::Reset,
    }
}

fn short_char(short: &Option<String>) -> Resettable<char> {
    match short {
        Some(text) => Resettable::Value(text.chars().next().unwrap().into()),
        None => Resettable::Reset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_runtime::engine::arg::Arg;
    use starlark::collections::SmallMap;
    use starlark::values::FrozenValue;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    static EMPTY: OnceLock<String> = OnceLock::new();
    fn empty_string() -> &'static String {
        EMPTY.get_or_init(String::new)
    }

    struct StubTask {
        name: String,
        group: Vec<String>,
        args: SmallMap<String, Arg>,
        path: PathBuf,
    }

    impl<'v> TaskLike<'v> for StubTask {
        fn args(&self) -> &SmallMap<String, Arg> {
            &self.args
        }
        fn summary(&self) -> &String {
            empty_string()
        }
        fn description(&self) -> &String {
            empty_string()
        }
        fn display_name(&self) -> String {
            String::new()
        }
        fn group(&self) -> &Vec<String> {
            &self.group
        }
        fn name(&self) -> String {
            self.name.clone()
        }
        fn path(&self) -> &PathBuf {
            &self.path
        }
        fn overrides(&self) -> Value<'v> {
            FrozenValue::new_none().to_value()
        }
        fn implementation(&self) -> Value<'v> {
            unimplemented!("test stub")
        }
        fn trait_type_ids(&self) -> Vec<u64> {
            vec![]
        }
    }

    fn stub_task(name: &str, group: &[&str], args: SmallMap<String, Arg>) -> StubTask {
        StubTask {
            name: name.to_owned(),
            group: group.iter().map(|s| s.to_string()).collect(),
            args,
            path: PathBuf::from("/repo/tasks/test.axl"),
        }
    }

    fn arg_string(default: &str) -> Arg {
        Arg::String {
            required: false,
            default: default.to_owned(),
            short: None,
            long: None,
            values: None,
            description: None,
        }
    }

    // ── Cmd::build ─────────────────────────────────────────────────────────

    #[test]
    fn build_renders_a_task_subcommand() {
        let mut args: SmallMap<String, Arg> = SmallMap::new();
        args.insert("greeting".to_owned(), arg_string("hello"));
        let t = stub_task("greet", &[], args);
        let cmd = Cmd {
            tasks: vec![&t as &dyn TaskLike],
            features: vec![],
            repo_root: Path::new("/repo"),
            modules: &[],
        };
        let root = cmd.build("0.0.0").expect("build ok");
        assert!(root.find_subcommand("greet").is_some());
    }

    #[test]
    fn build_groups_tasks_under_subcommand() {
        let t1 = stub_task("a", &["dev"], SmallMap::new());
        let t2 = stub_task("b", &["dev"], SmallMap::new());
        let cmd = Cmd {
            tasks: vec![&t1, &t2],
            features: vec![],
            repo_root: Path::new("/repo"),
            modules: &[],
        };
        let root = cmd.build("0.0.0").expect("build ok");
        let dev = root.find_subcommand("dev").expect("dev group present");
        assert!(dev.find_subcommand("a").is_some());
        assert!(dev.find_subcommand("b").is_some());
    }

    #[test]
    fn build_rejects_duplicate_task_names() {
        let t1 = stub_task("dup", &[], SmallMap::new());
        let t2 = stub_task("dup", &[], SmallMap::new());
        let cmd = Cmd {
            tasks: vec![&t1, &t2],
            features: vec![],
            repo_root: Path::new("/repo"),
            modules: &[],
        };
        match cmd.build("0.0.0") {
            Err(CmdError::NameConflict(name, ..)) => assert_eq!(name, "dup"),
            other => panic!("expected NameConflict, got {:?}", other),
        }
    }

    // ── Dispatch ───────────────────────────────────────────────────────────

    #[test]
    fn dispatch_extracts_task_id_and_key() {
        let t = stub_task("greet", &[], SmallMap::new());
        let cmd = Cmd {
            tasks: vec![&t],
            features: vec![],
            repo_root: Path::new("/repo"),
            modules: &[],
        };
        let root = cmd.build("0.0.0").expect("build ok");
        let matches = root
            .try_get_matches_from(["aspect", "greet", "--task-key=smoke"])
            .expect("parse ok");
        let dispatch = cmd.dispatch(matches).expect("dispatch ok");
        assert_eq!(dispatch.task_id, 0);
        assert_eq!(dispatch.task_key, "smoke");
        assert!(dispatch.task_uuid.is_none());
    }

    #[test]
    fn dispatch_auto_generates_task_key_when_absent() {
        let t = stub_task("greet", &[], SmallMap::new());
        let cmd = Cmd {
            tasks: vec![&t],
            features: vec![],
            repo_root: Path::new("/repo"),
            modules: &[],
        };
        let root = cmd.build("0.0.0").expect("build ok");
        let matches = root
            .try_get_matches_from(["aspect", "greet"])
            .expect("parse ok");
        let dispatch = cmd.dispatch(matches).expect("dispatch ok");
        assert!(!dispatch.task_key.is_empty());
    }

    // ── Override merge (no heap path: no overrides applied) ────────────────

    #[test]
    fn task_args_merges_cli_value() {
        let mut args: SmallMap<String, Arg> = SmallMap::new();
        args.insert("greeting".to_owned(), arg_string("hello"));
        let t = stub_task("greet", &[], args);
        let cmd = Cmd {
            tasks: vec![&t],
            features: vec![],
            repo_root: Path::new("/repo"),
            modules: &[],
        };
        let root = cmd.build("0.0.0").expect("build ok");
        let matches = root
            .try_get_matches_from(["aspect", "greet", "--greeting=hi"])
            .expect("parse ok");
        let dispatch = cmd.dispatch(matches).expect("dispatch ok");

        Heap::temp(|heap| {
            let merged = dispatch.task_args(&t, heap);
            let v = merged.get("greeting").expect("present");
            assert_eq!(v.unpack_str(), Some("hi"));
        });
    }

    #[test]
    fn clap_id_for_task_is_bare_name() {
        let arg = arg_string("");
        assert_eq!(clap_id(Scope::Task, "upload_bucket", &arg), "upload_bucket");
    }

    #[test]
    fn clap_id_for_feature_uses_prefix() {
        let arg = arg_string("");
        assert_eq!(
            clap_id(Scope::Feature("artifact-upload"), "mode", &arg),
            "artifact-upload:mode"
        );
    }

    #[test]
    fn clap_id_for_feature_respects_long_override() {
        let arg = Arg::String {
            required: false,
            default: String::new(),
            short: None,
            long: Some("custom-flag".to_owned()),
            values: None,
            description: None,
        };
        assert_eq!(
            clap_id(Scope::Feature("artifact-upload"), "mode", &arg),
            "custom-flag"
        );
    }
}
