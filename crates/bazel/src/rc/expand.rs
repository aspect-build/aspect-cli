use super::{Bazelrc, BazelrcError, RcOption};

/// Expand all `--config=` flags for the given command, returning a flat ordered list.
///
/// CLI flags are passed during `Bazelrc::new` construction and stored as `always` options,
/// so they are already included via `options_for`. No separate cli_args parameter is needed.
///
/// Each `RcOption` in the result preserves its `version_condition`. When a version-gated
/// `--config=` flag triggers expansion, its condition is inherited by expanded entries that
/// have no condition of their own.
///
/// An expanded entry also takes the `command` and `source_index` of the `--config=` request
/// that pulled it in, not of the `{section}:{config}` line that defined it: Bazel expands a
/// config in place, so its options rank where the request sits. `partition_expand_all`
/// renders by section and source, so without this a caller's `--config=ci` would demote
/// `build:ci` options to rc priority, and an rc `test --config=ci` would file them under
/// `build`, where the `test` section's own entries beat them. Two kinds of entry keep the
/// section they were written in:
///
/// - those defined under `common:{config}`, because `common` is what lets them reach
///   commands that reject them;
/// - those of the implicit `--config=<os>` that `--enable_platform_specific_config` adds,
///   which is no request of the user's and so has no section of its own to give.
///
/// `skip_config_if_missing` lists config names that should be silently dropped when no
/// matching section exists, rather than returning an `UndefinedConfig` error.
pub(crate) fn expand_configs(
    rc: &Bazelrc,
    command: &str,
    skip_config_if_missing: &[&str],
) -> Result<Vec<RcOption>, BazelrcError> {
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
    Expander {
        rc,
        command,
        implicit_platform_config: has_platform_config,
        skip_config_if_missing,
    }
    .expand(&base, &mut Vec::new(), &mut result)?;

    Ok(result)
}

/// The config name the option at `index` requests, and how many options the
/// request spans: 1 for `--config=NAME`, 2 for the `--config NAME` form Bazel
/// equally accepts.
///
/// `None` when this is not a config request at all, which includes a trailing
/// `--config` with no value and a `--config -flag`: Bazel reports the missing
/// value better than we could, so those pass through untouched.
fn config_request(args: &[RcOption], index: usize) -> Option<(&str, usize)> {
    let value = args[index].value.as_str();
    if let Some(name) = value.strip_prefix("--config=") {
        return Some((name, 1));
    }
    if value != "--config" {
        return None;
    }
    let next = args.get(index + 1)?.value.as_str();
    (!next.starts_with('-')).then_some((next, 2))
}

/// Walk `args`, emitting non-config options in place and expanding config
/// requests recursively where they appear — Bazel's in-place expansion
/// semantics. See <https://bazel.build/versions/9.0.0/run/bazelrc#option-defaults>.
///
/// The invariants of one expansion pass live on [`Expander`]; only the ancestor
/// chain and the output accumulate as it recurses.
struct Expander<'a> {
    rc: &'a Bazelrc,
    command: &'a str,
    /// When true, an undefined config is silently skipped rather than an error.
    /// Used for the synthetic `--config=<os>` injected by
    /// `--enable_platform_specific_config`.
    implicit_platform_config: bool,
    skip_config_if_missing: &'a [&'a str],
}

impl Expander<'_> {
    fn expand(
        &self,
        args: &[RcOption],
        ancestor_chain: &mut Vec<String>,
        result: &mut Vec<RcOption>,
    ) -> Result<(), BazelrcError> {
        let mut index = 0;
        while index < args.len() {
            let Some((config_name, span)) = config_request(args, index) else {
                result.push(args[index].clone());
                index += 1;
                continue;
            };
            let implicit = self.implicit_platform_config && ancestor_chain.is_empty() && index == 0;
            let requester = (!implicit).then_some(&args[index]);
            self.expand_config(config_name, requester, ancestor_chain, result)?;
            index += span;
        }
        Ok(())
    }

    /// Expand one `--config=NAME` request in place, or skip it when the section
    /// is absent and that is tolerated.
    ///
    /// Expanded options inherit `requester`'s `version_condition` when they carry
    /// none of their own, so version-gated config sections propagate correctly,
    /// and its section and source unless defined under `common` (see
    /// [`expand_configs`]). `requester` is `None` for the implicit platform
    /// config, whose options keep everything they were written with.
    fn expand_config(
        &self,
        config_name: &str,
        requester: Option<&RcOption>,
        ancestor_chain: &mut Vec<String>,
        result: &mut Vec<RcOption>,
    ) -> Result<(), BazelrcError> {
        if ancestor_chain.iter().any(|seen| seen == config_name) {
            let mut cycle = ancestor_chain.clone();
            cycle.push(config_name.to_owned());
            return Err(BazelrcError::ConfigCycle { cycle });
        }

        // Every section the command reads contributes its `{section}:{config}` block.
        // Not first-match-wins, so `build:opt` and `test:opt` both apply to
        // `bazel test --config=opt`.
        let mut config_opts: Vec<RcOption> = Vec::new();
        for prefix in super::sections_for(self.command) {
            config_opts.extend(
                self.rc
                    .raw_options(&format!("{prefix}:{config_name}"))
                    .iter()
                    .cloned(),
            );
        }
        if config_opts.is_empty() {
            // The synthetic OS config from --enable_platform_specific_config is
            // silently skipped when no matching section exists (Bazel spec: "if
            // applicable"). An explicitly requested config still errors.
            let is_implicit_platform = self.implicit_platform_config
                && ancestor_chain.is_empty()
                && config_name == platform_config_name();
            if is_implicit_platform || self.skip_config_if_missing.contains(&config_name) {
                return Ok(());
            }
            return Err(BazelrcError::UndefinedConfig {
                command: self.command.to_owned(),
                name: config_name.to_owned(),
            });
        }

        if let Some(requester) = requester {
            for opt in &mut config_opts {
                if opt.version_condition.is_none() {
                    opt.version_condition = requester.version_condition.clone();
                }
                if opt.section() != "common" {
                    opt.command = requester.command.clone();
                    opt.source_index = requester.source_index;
                }
            }
        }

        ancestor_chain.push(config_name.to_owned());
        let expanded = self.expand(&config_opts, ancestor_chain, result);
        ancestor_chain.pop();
        expanded
    }
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
