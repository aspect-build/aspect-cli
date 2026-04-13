use axl_runtime::engine::arg::Arg;
use clap::builder::{PossibleValuesParser, Resettable, StyledStr};
use clap::{Arg as ClapArg, ArgAction, value_parser};

/// Build a Clap [`ClapArg`] from an [`Arg`] definition.
///
/// - `id` — the Clap internal identifier used to retrieve the value from `ArgMatches`
///   (e.g. `"upload_bucket"` for task args, `"artifact-upload-mode"` for feature args).
/// - `long_name` — the `--flag-name` shown on the CLI (e.g. `"upload-bucket"` or
///   `"artifact-upload-mode"`).
pub(crate) fn convert_arg(id: &str, long_name: &str, arg: &Arg) -> ClapArg {
    match arg {
        Arg::String {
            required,
            default,
            short,
            values,
            description,
            ..
        } => {
            // Clap requires 'static data for .long(), .value_name(), and .default_value(),
            // so we pass owned Strings throughout.
            let mut it = ClapArg::new(id.to_string())
                .long(long_name.to_string())
                .value_name(long_name.to_string())
                .short(short_option(short))
                .help(help_text(description))
                .required(*required)
                .default_value(default.clone());
            if let Some(values) = values {
                it = it.value_parser(PossibleValuesParser::new(values))
            } else {
                it = it.value_parser(value_parser!(String));
            }
            it
        }
        Arg::Boolean {
            required,
            default,
            short,
            description,
            ..
        } => ClapArg::new(id.to_string())
            .long(long_name.to_string())
            .value_name(long_name.to_string())
            .short(short_option(short))
            .help(help_text(description))
            .required(*required)
            // Use static string literals — no allocation, and Clap's bool parser expects lowercase.
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
        } => ClapArg::new(id.to_string())
            .long(long_name.to_string())
            .value_name(long_name.to_string())
            .short(short_option(short))
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
        } => ClapArg::new(id.to_string())
            .long(long_name.to_string())
            .value_name(long_name.to_string())
            .short(short_option(short))
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
            let mut it = ClapArg::new(id.to_string())
                .value_parser(value_parser!(String))
                .value_name(id.to_string())
                .help(help_text(description))
                .num_args(*minimum as usize..=*maximum as usize);
            if let Some(default) = default {
                it = it.default_values(default);
            }
            it
        }
        Arg::TrailingVarArgs { description } => ClapArg::new(id.to_string())
            .value_parser(value_parser!(String))
            .value_name(id.to_string())
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
            let mut it = ClapArg::new(id.to_string())
                .long(long_name.to_string())
                .value_name(long_name.to_string())
                .short(short_option(short))
                .help(help_text(description))
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(String));
            if !default.is_empty() {
                // Pass an iterator of cloned Strings — avoids allocating an intermediate Vec.
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
            let mut it = ClapArg::new(id.to_string())
                .long(long_name.to_string())
                .value_name(long_name.to_string())
                .short(short_option(short))
                .help(help_text(description))
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(bool))
                .num_args(0..=1)
                .default_missing_value("true");
            if !default.is_empty() {
                // Static string literals — no allocation, and Clap's bool parser expects lowercase.
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
            let mut it = ClapArg::new(id.to_string())
                .long(long_name.to_string())
                .value_name(long_name.to_string())
                .short(short_option(short))
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
            let mut it = ClapArg::new(id.to_string())
                .long(long_name.to_string())
                .value_name(long_name.to_string())
                .short(short_option(short))
                .help(help_text(description))
                .action(ArgAction::Append)
                .required(*required)
                .value_parser(value_parser!(u32));
            if !default.is_empty() {
                it = it.default_values(default.iter().map(|u| u.to_string()));
            }
            it
        }
        Arg::Custom { .. } => {
            // Custom args are config.axl-only and never exposed on the CLI.
            // convert_arg should never be called with a Custom arg.
            unreachable!("Custom args are not CLI-exposed")
        }
    }
}

fn help_text(description: &Option<String>) -> Resettable<StyledStr> {
    match description {
        Some(text) => Resettable::Value(text.into()),
        None => Resettable::Reset,
    }
}

fn short_option(short: &Option<String>) -> Resettable<char> {
    match short {
        Some(text) => Resettable::Value(text.chars().nth(0).unwrap().into()),
        None => Resettable::Reset,
    }
}
