use axl_runtime::engine::task_arg::TaskArg;
use clap::{Arg, value_parser};

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
            .default_missing_value(default.to_string()),
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
        TaskArg::Positional { minimum, maximum } => Arg::new(name)
            .value_parser(value_parser!(String))
            .value_name(name)
            .num_args(minimum.to_owned() as usize..=maximum.to_owned() as usize),
        TaskArg::TrailingVarArgs => Arg::new(name)
            .value_parser(value_parser!(String))
            .value_name(name)
            .allow_hyphen_values(true)
            .last(true)
            .num_args(0..),
    }
}
