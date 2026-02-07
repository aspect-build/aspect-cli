use axl_runtime::engine::task_arg::TaskArg;
use clap::{Arg, ArgAction, value_parser};

pub(crate) fn convert_arg(name: &String, arg: &TaskArg) -> Arg {
    match arg {
        TaskArg::String { required, default } => Arg::new(name)
            .long(name)
            .value_name(name)
            .required(required.to_owned())
            .default_value(default.to_string())
            .value_parser(value_parser!(String)),
        TaskArg::Boolean { required, default } => Arg::new(name)
            .long(name)
            .value_name(name)
            .required(required.to_owned())
            .default_value(default.to_string())
            .value_parser(value_parser!(bool))
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value("true"),
        TaskArg::Int { required, default } => Arg::new(name)
            .long(name)
            .value_name(name)
            .required(required.to_owned())
            .default_value(default.to_string())
            .value_parser(value_parser!(i32)),
        TaskArg::UInt { required, default } => Arg::new(name)
            .long(name)
            .value_name(name)
            .required(required.to_owned())
            .default_value(default.to_string())
            .value_parser(value_parser!(u32)),
        TaskArg::Positional {
            minimum,
            maximum,
            default,
        } => {
            let mut it = Arg::new(name)
                .value_parser(value_parser!(String))
                .value_name(name)
                .num_args(minimum.to_owned() as usize..=maximum.to_owned() as usize);
            if let Some(default) = default {
                it = it.default_values(default);
            }
            it
        }
        TaskArg::TrailingVarArgs => Arg::new(name)
            .value_parser(value_parser!(String))
            .value_name(name)
            .allow_hyphen_values(true)
            .last(true)
            .num_args(0..),
        TaskArg::StringList { required, default } => {
            let mut it = Arg::new(name)
                .long(name)
                .value_name(name)
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(String));
            if !default.is_empty() {
                it = it.default_values(default.clone());
            }
            it
        }
        TaskArg::BooleanList { required, default } => {
            let mut it = Arg::new(name)
                .long(name)
                .value_name(name)
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
        TaskArg::IntList { required, default } => {
            let mut it = Arg::new(name)
                .long(name)
                .value_name(name)
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(i32));
            if !default.is_empty() {
                let default: Vec<String> = default.iter().map(|&i| i.to_string()).collect();
                it = it.default_values(default);
            }
            it
        }
        TaskArg::UIntList { required, default } => {
            let mut it = Arg::new(name)
                .long(name)
                .value_name(name)
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
