use std::{collections::HashMap, path::PathBuf};

use axl_runtime::engine::task::{TaskLike, MAX_TASK_GROUPS};
use clap::{Arg, ArgMatches, Command};
use thiserror::Error;

const TASK_COMMAND_PATH_ID: &'static str = "@@@__AXL__PATH__@@@";
const TASK_COMMAND_SYMBOL_ID: &'static str = "@@@__AXL__SYMBOL__@@@";

// Clap's help generation sorts by (display_order, name)—-equal display_order values fall back to name-based sorting.
const TASK_COMMAND_DISPLAY_ORDER: usize = 0;
const TASK_GROUP_DISPLAY_ORDER: usize = 1;
pub const BUILTIN_COMMAND_DISPLAY_ORDER: usize = 2;

#[derive(Default)]
pub struct CommandTree {
    // mapping of task subgroup to tree that contains subcommand.
    pub(crate) subgroups: HashMap<String, CommandTree>,
    // mapping of task name to the path that defines the task
    pub(crate) tasks: HashMap<String, Command>,
}

#[derive(Error, Debug)]
pub enum TreeError {
    #[error("task {0:?} in group {1:?} (defined in {2:?}) conflicts with a previously defined group")]
    TaskGroupConflict(String, Vec<String>, PathBuf),

    #[error("group {0:?} from task {1:?} in group {2:?} (defined in {3:?}) conflicts with a previously defined task")]
    GroupConflictTask(String, String, Vec<String>, PathBuf),

    #[error("task {0:?} in group {1:?} (defined in {2:?}) conflicts with a previously defined task")]
    TaskConflict(String, Vec<String>, PathBuf),

    #[error("task {0:?} (defined in {1:?}) cannot have more than {2:?} group levels")]
    TooManyGroups(String, PathBuf, usize),

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
        path: &PathBuf,
        cmd: Command,
    ) -> Result<(), TreeError> {
        if group.len() > MAX_TASK_GROUPS {
            // This error is a defence-in-depth as the task evaluator should check for MAX_TASK_GROUPS
            return Err(TreeError::TooManyGroups(name.to_string(), path.clone(), MAX_TASK_GROUPS));
        }
        if subgroup.is_empty() {
            if self.subgroups.contains_key(name) {
                return Err(TreeError::TaskGroupConflict(name.to_string(), group.to_vec(), path.clone()));
            }
            if self.tasks.insert(name.to_string(), cmd).is_some() {
                return Err(TreeError::TaskConflict(name.to_string(), group.to_vec(), path.clone()));
            }
        } else {
            let first = &subgroup[0];
            if self.tasks.contains_key(first) {
                return Err(TreeError::GroupConflictTask(first.clone(), name.to_string(), group.to_vec(), path.clone()));
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
            let mut subcmd = Command::new(name.clone())
                // customize the subcommands section title to "Tasks:"
                .subcommand_help_heading("Tasks")
                // customize the usage string to use <TASK>
                .subcommand_value_name("TASK")
                .about(format!("\x1b[3m{}\x1b[0m task group", name))
                .display_order(TASK_GROUP_DISPLAY_ORDER);
            subcmd = subtree.as_command(subcmd, &group)?;
            current = current.subcommand(subcmd);
        }

        for (name, subcmd) in &self.tasks {
            if current.find_subcommand(name).is_some() {
                return Err(TreeError::TaskCommandConflict(name.to_string(), group.to_vec()));
            }
            current = current.subcommand(subcmd);
        }

        // Require subcommand if are subgroups or tasks
        if !self.subgroups.is_empty() || !self.tasks.is_empty() {
            current = current.subcommand_required(true);
        }

        Ok(current)
    }

    pub fn get_task_path(&self, matches: &ArgMatches) -> String {
        assert!(matches.contains_id(TASK_COMMAND_PATH_ID));
        matches.get_one::<String>(TASK_COMMAND_PATH_ID).unwrap().clone()
    }

    pub fn get_task_symbol(&self, matches: &ArgMatches) -> String {
        assert!(matches.contains_id(TASK_COMMAND_SYMBOL_ID));
        matches.get_one::<String>(TASK_COMMAND_SYMBOL_ID).unwrap().clone()
    }
}

pub fn make_command_from_task(
    name: &String,
    defined_in: &str,
    path: &PathBuf,
    symbol: &String,
    task: &dyn TaskLike<'_>,
) -> Command {
    // Generate a default task description if none was provided by task
    let about = if task.description().is_empty() {
        format!(
            "\x1b[3m{}\x1b[0m task defined in \x1b[3m{}\x1b[0m",
            name, defined_in,
        )
    } else {
        task.description().clone()
    };

    let mut cmd = Command::new(name)
        .about(about)
        .display_order(TASK_COMMAND_DISPLAY_ORDER)
        .arg(
            Arg::new(TASK_COMMAND_PATH_ID)
                .long(TASK_COMMAND_PATH_ID)
                .hide(true)
                .hide_default_value(true)
                .hide_short_help(true)
                .hide_possible_values(true)
                .hide_long_help(true)
                .default_value(path.as_os_str().to_string_lossy().to_string()),
        )
        .arg(
            Arg::new(TASK_COMMAND_SYMBOL_ID)
                .long(TASK_COMMAND_SYMBOL_ID)
                .hide(true)
                .hide_default_value(true)
                .hide_short_help(true)
                .hide_possible_values(true)
                .hide_long_help(true)
                .default_value(symbol),
        );

    for (name, arg) in task.args() {
        let arg = crate::flags::convert_arg(&name, arg);
        cmd = cmd.arg(arg);
    }

    cmd
}
