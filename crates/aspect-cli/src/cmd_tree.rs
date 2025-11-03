use std::{collections::HashMap, path::PathBuf};

use axl_runtime::engine::task::{TaskLike, MAX_TASK_GROUPS};
use clap::{Arg, ArgMatches, Command};
use thiserror::Error;

const COMMAND_PATH_ID: &'static str = "@@@__AXL__PATH__@@@";

#[derive(Default)]
pub struct CommandTree {
    // mapping of task group to tree that contains subcommand.
    pub(crate) subgroups: HashMap<String, CommandTree>,
    // mapping of task name to the path that defines the task
    pub(crate) subtasks: HashMap<String, Command>,
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
            if self.subtasks.insert(name.to_string(), cmd).is_some() {
                return Err(TreeError::TaskConflict(name.to_string(), group.to_vec(), path.clone()));
            }
        } else {
            let first = &subgroup[0];
            if self.subtasks.contains_key(first) {
                return Err(TreeError::GroupConflictTask(first.clone(), name.to_string(), group.to_vec(), path.clone()));
            }
            let subtree = self.subgroups.entry(first.clone()).or_default();
            subtree.insert(name, group, &subgroup[1..], path, cmd)?;
        }
        Ok(())
    }

    pub fn as_command(&self, mut current: Command) -> Command {
        // Collect all subcommand names (groups and tasks) and sort them alphabetically

        for (name, subtree) in &self.subgroups {
            let mut subcmd = Command::new(name.clone());
            subcmd = subtree.as_command(subcmd);
            // If the group has subcommands or tasks, require a subcommand
            if !subtree.subgroups.is_empty() || !subtree.subtasks.is_empty() {
                subcmd = subcmd.subcommand_required(true);
            }
            current = current.subcommand(subcmd);
        }

        for (_, subcmd) in &self.subtasks {
            current = current.subcommand(subcmd);
        }

        // For the root command, require subcommand if there are any
        if !self.subgroups.is_empty() || !self.subtasks.is_empty() {
            current = current.subcommand_required(true);
        }

        current
    }

    pub fn get_task_path(&self, matches: &ArgMatches) -> String {
        assert!(matches.contains_id(COMMAND_PATH_ID));
        matches.get_one::<String>(COMMAND_PATH_ID).unwrap().clone()
    }
}

pub fn make_command(
    name: &String,
    defined_in: &str,
    path: &PathBuf,
    task: &dyn TaskLike<'_>,
) -> Command {
    let about = if task.description().is_empty() {
        format!(
            "\x1b[3m{}\x1b[0m task defined in \x1b[3m{}\x1b[0m",
            name, defined_in,
        )
    } else {
        task.description().clone()
    };

    let mut subcmd = Command::new(name).about(about).arg(
        Arg::new(COMMAND_PATH_ID)
            .long(COMMAND_PATH_ID)
            .hide(true)
            .hide_default_value(true)
            .hide_short_help(true)
            .hide_possible_values(true)
            .hide_long_help(true)
            .default_value(path.as_os_str().to_string_lossy().to_string()),
    );

    for (name, arg) in task.args() {
        let arg = crate::flags::convert_arg(&name, arg);
        subcmd = subcmd.arg(arg);
    }

    subcmd
}
