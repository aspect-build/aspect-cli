use std::{collections::HashMap, path::PathBuf};

use axl_runtime::engine::task::TaskLike;
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
    #[error("task '{0}' clashes with a subgroup")]
    TaskSubgroupConflict(String),

    #[error("group name '{0}' clashes with a task ")]
    GroupConflictTask(String),

    #[error("duplicate task '{0}'")]
    DuplicateTask(String),

    #[error("task '{0}' defined in {1:?} cannot have more than 5 group levels.")]
    TooManyGroups(String, PathBuf),
}

impl CommandTree {
    pub fn insert(
        &mut self,
        group: &[String],
        name: String,
        path: &PathBuf,
        cmd: Command,
    ) -> Result<(), TreeError> {
        if group.len() > 5 {
            return Err(TreeError::TooManyGroups(name, path.clone()));
        }
        if group.is_empty() {
            if self.subgroups.contains_key(&name) {
                return Err(TreeError::TaskSubgroupConflict(name.clone()));
            }
            if self.subtasks.insert(name.clone(), cmd).is_some() {
                return Err(TreeError::DuplicateTask(name.clone()));
            }
        } else {
            let first = &group[0];
            if self.subtasks.contains_key(first) {
                return Err(TreeError::GroupConflictTask(first.clone()));
            }
            let subtree = self.subgroups.entry(first.clone()).or_default();
            subtree.insert(&group[1..], name, path, cmd)?;
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
