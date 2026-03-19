use crate::{BazelRC, BazelRcError, RcOption};

/// Expand all `--config=` flags for the given command, returning a flat ordered list.
///
/// CLI flags are passed during `BazelRC::new` construction and stored as `always` options,
/// so they are already included via `options_for`. No separate cli_args parameter is needed.
///
/// Each `RcOption` in the result preserves its `version_condition`. When a version-gated
/// `--config=` flag triggers expansion, its condition is inherited by expanded entries that
/// have no condition of their own.
pub(crate) fn expand_configs(rc: &BazelRC, command: &str) -> Result<Vec<RcOption>, BazelRcError> {
    let mut base: Vec<RcOption> = rc.options_for(command).into_iter().cloned().collect();

    // Check for --enable_platform_specific_config
    let has_platform_config = base
        .iter()
        .any(|o| o.value == "--enable_platform_specific_config");
    if has_platform_config {
        let os_name = platform_config_name();
        base.insert(
            0,
            RcOption {
                value: format!("--config={os_name}"),
                command: "always".to_owned(),
                source_index: 0,
                version_condition: None,
            },
        );
    }

    let mut result = Vec::new();
    expand_args(
        rc,
        command,
        &base,
        &mut Vec::new(),
        &mut result,
        has_platform_config,
    )?;

    Ok(result)
}

fn expand_args(
    rc: &BazelRC,
    command: &str,
    args: &[RcOption],
    ancestor_chain: &mut Vec<String>,
    result: &mut Vec<RcOption>,
    // When true, an undefined config is silently skipped rather than an error.
    // Used for the synthetic --config=<os> injected by --enable_platform_specific_config.
    implicit_platform_config: bool,
) -> Result<(), BazelRcError> {
    for opt in args {
        if let Some(config_name) = opt.value.strip_prefix("--config=") {
            // Cycle detection
            if ancestor_chain.contains(&config_name.to_owned()) {
                let mut cycle = ancestor_chain.clone();
                cycle.push(config_name.to_owned());
                return Err(BazelRcError::ConfigCycle { cycle });
            }

            // Look up {command}:{config_name}
            let key = format!("{command}:{config_name}");
            let config_opts: Vec<RcOption> = match rc.raw_options(&key) {
                opts if !opts.is_empty() => opts.to_vec(),
                _ => {
                    // Also check common:{config_name} and always:{config_name}
                    let always_key = format!("always:{config_name}");
                    let common_key = format!("common:{config_name}");
                    let combined: Vec<RcOption> = rc
                        .raw_options(&always_key)
                        .iter()
                        .chain(rc.raw_options(&common_key).iter())
                        .cloned()
                        .collect();
                    if combined.is_empty() {
                        // The synthetic OS config from --enable_platform_specific_config is
                        // silently skipped when no matching section exists (Bazel spec: "if
                        // applicable"). Explicitly-requested --config= still errors.
                        let is_implicit_platform = implicit_platform_config
                            && ancestor_chain.is_empty()
                            && config_name == platform_config_name();
                        if is_implicit_platform {
                            continue;
                        }
                        return Err(BazelRcError::UndefinedConfig {
                            command: command.to_owned(),
                            name: config_name.to_owned(),
                        });
                    }
                    combined
                }
            };

            // Expanded options inherit the triggering flag's version_condition when they
            // have none of their own, so version-gated config sections propagate correctly.
            let parent_condition = opt.version_condition.clone();
            let inherited: Vec<RcOption> = config_opts
                .into_iter()
                .map(|mut o| {
                    if o.version_condition.is_none() {
                        o.version_condition = parent_condition.clone();
                    }
                    o
                })
                .collect();

            ancestor_chain.push(config_name.to_owned());
            expand_args(
                rc,
                command,
                &inherited,
                ancestor_chain,
                result,
                implicit_platform_config,
            )?;
            ancestor_chain.pop();
        } else {
            result.push(opt.clone());
        }
    }
    Ok(())
}

/// Map std::env::consts::OS to Bazel's platform config name.
fn platform_config_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        other => other,
    }
}
