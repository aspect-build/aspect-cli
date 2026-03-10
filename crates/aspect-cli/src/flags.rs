use axl_runtime::engine::task_arg::TaskArg;
use clap::builder::{Resettable, StyledStr};
use clap::{Arg, ArgAction, value_parser};

pub(crate) fn convert_arg(name: &String, arg: &TaskArg) -> Arg {
    match arg {
        TaskArg::String {
            required,
            default,
            description,
        } => Arg::new(name)
            .long(name)
            .value_name(name)
            .help(help_text(description))
            .required(required.to_owned())
            .default_value(default.to_string())
            .value_parser(value_parser!(String)),
        TaskArg::Boolean {
            required,
            default,
            description,
        } => Arg::new(name)
            .long(name)
            .value_name(name)
            .help(help_text(description))
            .required(required.to_owned())
            .default_value(default.to_string())
            .value_parser(value_parser!(bool))
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value("true"),
        TaskArg::Int {
            required,
            default,
            description,
        } => Arg::new(name)
            .long(name)
            .value_name(name)
            .help(help_text(description))
            .required(required.to_owned())
            .default_value(default.to_string())
            .value_parser(value_parser!(i32)),
        TaskArg::UInt {
            required,
            default,
            description,
        } => Arg::new(name)
            .long(name)
            .value_name(name)
            .help(help_text(description))
            .required(required.to_owned())
            .default_value(default.to_string())
            .value_parser(value_parser!(u32)),
        TaskArg::Positional {
            minimum,
            maximum,
            default,
            description,
        } => {
            let mut it = Arg::new(name)
                .value_parser(value_parser!(String))
                .value_name(name)
                .help(help_text(description))
                .num_args(minimum.to_owned() as usize..=maximum.to_owned() as usize);
            if let Some(default) = default {
                it = it.default_values(default);
            }
            it
        }
        TaskArg::TrailingVarArgs { description } => Arg::new(name)
            .value_parser(value_parser!(String))
            .value_name(name)
            .help(help_text(description))
            .allow_hyphen_values(true)
            .last(true)
            .num_args(0..),
        TaskArg::StringList {
            required,
            default,
            description,
        } => {
            let mut it = Arg::new(name)
                .long(name)
                .value_name(name)
                .help(help_text(description))
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(String));
            if !default.is_empty() {
                it = it.default_values(default.clone());
            }
            it
        }
        TaskArg::BooleanList {
            required,
            default,
            description,
        } => {
            let mut it = Arg::new(name)
                .long(name)
                .value_name(name)
                .help(help_text(description))
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(bool))
                .num_args(0..=1)
                .default_missing_value("true");
            if !default.is_empty() {
                let default: Vec<String> = default.iter().map(|&b| b.to_string()).collect();
                it = it.default_values(default);
            }
            it
        }
        TaskArg::IntList {
            required,
            default,
            description,
        } => {
            let mut it = Arg::new(name)
                .long(name)
                .value_name(name)
                .help(help_text(description))
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(i32));
            if !default.is_empty() {
                let default: Vec<String> = default.iter().map(|&i| i.to_string()).collect();
                it = it.default_values(default);
            }
            it
        }
        TaskArg::UIntList {
            required,
            default,
            description,
        } => {
            let mut it = Arg::new(name)
                .long(name)
                .value_name(name)
                .help(help_text(description))
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(u32));
            if !default.is_empty() {
                let default: Vec<String> = default.iter().map(|&u| u.to_string()).collect();
                it = it.default_values(default);
            }
            it
        }
    }
}

fn help_text(description: &Option<String>) -> Resettable<StyledStr> {
    match description {
        Some(text) => Resettable::Value(text.into()),
        None => Resettable::Reset,
    }
}
