use std::collections::HashMap;

use axl_runtime::engine::arg::Arg;
use axl_runtime::engine::task::{MAX_TASK_GROUPS, TaskLike};
use axl_runtime::engine::types::feature::to_display_name;
use clap::{Arg as ClapArg, ArgMatches, Command, value_parser};
use starlark::collections::SmallMap;
use thiserror::Error;

const TASK_ID: &'static str = "@@@$'__AXL_TASK_ID__'$@@@";

// Clap's help generation sorts by (display_order, name)—-equal display_order values fall back to name-based sorting.
const TASK_COMMAND_DISPLAY_ORDER: usize = 0;
const TASK_GROUP_DISPLAY_ORDER: usize = 1;

#[derive(Default)]
pub struct CommandTree {
    // mapping of task subgroup to tree that contains subcommand.
    pub(crate) subgroups: HashMap<String, CommandTree>,
    // mapping of task name to the path that defines the task
    pub(crate) tasks: HashMap<String, Command>,
}

#[derive(Error, Debug)]
pub enum TreeError {
    #[error(
        "task {0:?} in group {1:?} (defined in {2:?}) conflicts with a previously defined group"
    )]
    TaskGroupConflict(String, Vec<String>, String),

    #[error(
        "group {0:?} from task {1:?} in group {2:?} (defined in {3:?}) conflicts with a previously defined task"
    )]
    GroupConflictTask(String, String, Vec<String>, String),

    #[error(
        "task {0:?} in group {1:?} (defined in {2:?}) conflicts with a previously defined task"
    )]
    TaskConflict(String, Vec<String>, String),

    #[error("task {0:?} (defined in {1:?}) cannot have more than {2:?} group levels")]
    TooManyGroups(String, String, usize),

    #[error("task {0:?} in group {1:?} conflicts with a previously defined command")]
    TaskCommandConflict(String, Vec<String>),

    #[error("group {0:?} conflicts with a previously defined command")]
    GroupCommandConflict(Vec<String>),
}

impl CommandTree {
    pub fn insert(
        &mut self,
        name: &str,
        group: &[String],
        subgroup: &[String],
        path: &str,
        cmd: Command,
    ) -> Result<(), TreeError> {
        if group.len() > MAX_TASK_GROUPS {
            // This error is a defence-in-depth as the task evaluator should check for MAX_TASK_GROUPS
            return Err(TreeError::TooManyGroups(
                name.to_string(),
                path.to_owned(),
                MAX_TASK_GROUPS,
            ));
        }
        if subgroup.is_empty() {
            if self.subgroups.contains_key(name) {
                return Err(TreeError::TaskGroupConflict(
                    name.to_string(),
                    group.to_vec(),
                    path.to_owned(),
                ));
            }
            if self.tasks.insert(name.to_string(), cmd).is_some() {
                return Err(TreeError::TaskConflict(
                    name.to_string(),
                    group.to_vec(),
                    path.to_owned(),
                ));
            }
        } else {
            let first = &subgroup[0];
            if self.tasks.contains_key(first) {
                return Err(TreeError::GroupConflictTask(
                    first.clone(),
                    name.to_string(),
                    group.to_vec(),
                    path.to_owned(),
                ));
            }
            let subtree = self.subgroups.entry(first.clone()).or_default();
            subtree.insert(name, group, &subgroup[1..], path, cmd)?;
        }
        Ok(())
    }

    pub fn as_command(&self, mut current: Command, group: &[String]) -> Result<Command, TreeError> {
        // Collect all subcommand names (groups and tasks) and sort them alphabetically

        for (name, subtree) in &self.subgroups {
            let mut group = group.to_vec();
            group.push(name.clone());
            if current.find_subcommand(name).is_some() {
                return Err(TreeError::GroupCommandConflict(group.to_vec()));
            }

            // Build a custom help_template for this group with separate Tasks / Task Groups sections
            let sub_group_names = subtree.group_names();
            let mut template = String::from("{about-with-newline}\n{usage-heading} {usage}");

            if !subtree.tasks.is_empty() {
                template.push_str("\n\n\x1b[1;4mTasks:\x1b[0m\n{subcommands}");
            }

            if !sub_group_names.is_empty() {
                let max_len = sub_group_names.iter().map(|n| n.len()).max().unwrap_or(0);
                template.push_str("\n\n\x1b[1;4mTask Groups:\x1b[0m\n");
                for gname in &sub_group_names {
                    let padding = " ".repeat(max_len - gname.len() + 2);
                    template.push_str(&format!("  \x1b[1m{}\x1b[0m{}task group\n", gname, padding));
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
            subcmd = subtree.as_command(subcmd, &group)?;
            current = current.subcommand(subcmd);
        }

        for (name, subcmd) in &self.tasks {
            if current.find_subcommand(name).is_some() {
                return Err(TreeError::TaskCommandConflict(
                    name.to_string(),
                    group.to_vec(),
                ));
            }
            current = current.subcommand(subcmd);
        }

        // Print help if no subcommand is given (instead of erroring)
        if !self.subgroups.is_empty() || !self.tasks.is_empty() {
            current = current.arg_required_else_help(true);
        }

        Ok(current)
    }

    /// Returns sorted list of top-level task group names.
    pub fn group_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.subgroups.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }

    pub fn get_task_id(&self, matches: &ArgMatches) -> usize {
        assert!(matches.contains_id(TASK_ID));
        matches.get_one::<usize>(TASK_ID).unwrap().to_owned()
    }
}

/// Build a Clap subcommand for a task, including any args contributed by active features.
///
/// `feature_args` is a slice of `(args_map, heading, description_line, prefix)` tuples —
/// one per feature that declares CLI args. Each feature's args are prefixed with the
/// feature's kebab name (`prefix`) and grouped under a Clap help heading.
///
/// `effective_defaults` maps CLI arg names to their post-config.axl effective default
/// strings, overriding the task definition defaults in `--help`.
pub fn make_command_from_task(
    name: &String,
    defined_in: &str,
    indice: String,
    task: &dyn TaskLike<'_>,
    feature_args: &[(SmallMap<String, Arg>, String, String, String)],
    effective_defaults: &HashMap<String, Vec<String>>,
) -> Command {
    let task_display = if !task.display_name().is_empty() {
        task.display_name().clone()
    } else {
        to_display_name(name)
    };

    let context_line = format!(
        "\x1b[3m{}\x1b[0m task defined in \x1b[3m{}\x1b[0m",
        name, defined_in,
    );
    // `about` — single-line shown in the task list (and in --help when no custom template).
    // We avoid `long_about`/`before_long_help`/`after_long_help` entirely: setting any of those
    // switches Clap into "long help" mode for `--help`, which renders all option descriptions on
    // separate lines with extra blank lines between flags (verbose format).
    //
    // Instead, when there is a description or a summary+context to show in `--help`, we build a
    // custom help_template that hardcodes the header text and keeps compact option rendering.
    //
    //   summary  description  | about (list)   --help header
    //   -------  -----------  | ------------   -------------
    //   unset    unset        | context_line   (no custom template — about reused)
    //   set      unset        | summary        summary + blank + context_line
    //   set      set          | summary        description + blank + context_line
    //   unset    set          | context_line   description + blank + context_line
    let about = if task.summary().is_empty() {
        context_line.clone()
    } else {
        task.summary().clone()
    };

    // Build the header shown at the top of `--help`, or None to use the default (about).
    let help_header: Option<String> = if task.summary().is_empty() && task.description().is_empty()
    {
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
            ClapArg::new(TASK_ID)
                .long(TASK_ID)
                .hide(true)
                .hide_default_value(true)
                .hide_short_help(true)
                .hide_possible_values(true)
                .hide_long_help(true)
                .value_parser(value_parser!(usize))
                .default_value(indice),
        );

    for (arg_name, arg) in task.cli_args() {
        let heading = format!("{} Options", task_display);
        let long_name = match arg.long_override() {
            Some(lo) => lo.to_string(),
            None => arg_name.replace('_', "-"),
        };
        let mut clap_arg =
            crate::flags::convert_arg(arg_name, &long_name, arg).help_heading(heading);
        if let Some(effective) = effective_defaults.get(arg_name) {
            clap_arg = match arg {
                Arg::StringList { .. }
                | Arg::BooleanList { .. }
                | Arg::IntList { .. }
                | Arg::UIntList { .. }
                | Arg::TrailingVarArgs { .. } => clap_arg.default_values(effective.clone()),
                _ => {
                    // Scalar args always produce a single-element vec; use first() defensively.
                    if let Some(v) = effective.first() {
                        clap_arg.default_value(v.clone())
                    } else {
                        clap_arg
                    }
                }
            };
        }
        cmd = cmd.arg(clap_arg);
    }

    for (args, heading, description_line, prefix) in feature_args {
        let full_heading = if description_line.is_empty() {
            heading.clone()
        } else {
            // Clap always appends ":\n" after the heading string, so the colon lands
            // at the end of the description line. We manually add the colon after the
            // section title on the first line to keep it looking like a normal heading.
            format!("{}:\n{}", heading, description_line)
        };
        for (arg_name, arg) in args.iter() {
            // If the arg has an explicit long override, use it directly (no prefix).
            // Otherwise prefix with the feature's kebab name so args never collide:
            // `mode` in `artifact-upload` becomes `--artifact-upload:mode`.
            let clap_id = if let Some(lo) = arg.long_override() {
                lo.to_string()
            } else {
                let long_form = arg_name.replace('_', "-");
                if prefix.is_empty() {
                    long_form
                } else {
                    format!("{}:{}", prefix, long_form)
                }
            };
            let clap_arg = crate::flags::convert_arg(&clap_id, &clap_id, arg)
                .help_heading(full_heading.clone());
            cmd = cmd.arg(clap_arg);
        }
    }

    // Apply a custom help template when we have header content beyond the one-line `about`.
    // The template hardcodes the header so Clap never enters "long help" mode, keeping
    // option rendering compact (descriptions inline, no blank lines between flags).
    if let Some(header) = help_header {
        cmd = cmd.help_template(format!(
            "{header}\n\n{{usage-heading}} {{usage}}\n\n{{all-args}}\n"
        ));
    }

    cmd
}
