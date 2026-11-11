use super::{Bazelrc, BazelrcError, RcOption};
use crate::command::flags::bool_setting;

/// The option that adds the host OS's config, as `--config=<os>` would.
const PLATFORM_CONFIG_OPTION: &str = "enable_platform_specific_config";

/// The host OS's config name. Rust's names for the OSes Bazel names (`linux`,
/// `macos`, `windows`, `freebsd`, `openbsd`) are Bazel's own.
const PLATFORM_CONFIG: &str = std::env::consts::OS;

/// Expand all `--config=` flags for the given command, returning a flat ordered list.
///
/// Caller flags, passed to `Bazelrc::new`, are already among `options_for`'s options, under
/// the section each names (`always` when none).
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
/// `build`, where the `test` section's own entries beat them. Entries defined under
/// `common:{config}` keep `common`, because `common` is what lets them reach commands that
/// reject them.
///
/// The platform config applies when the last `--enable_platform_specific_config` setting,
/// in any boolean spelling and wherever it came from (a config included), turns it on and
/// some section the command reads defines `<os>`, as in Bazel's `ConfigExpander`. A
/// version-gated setting counts only where its condition holds at the configured Bazel
/// version, as when the options are rendered. Also as in Bazel, the platform config expands
/// in place of that setting: right after it, with its section, source and version
/// condition, as if it were `--config=<os>`.
///
/// `skip_config_if_missing` lists config names that should be silently dropped when no
/// matching section exists, rather than returning an `UndefinedConfig` error.
pub(crate) fn expand_configs(
    rc: &Bazelrc,
    command: &str,
    skip_config_if_missing: &[&str],
) -> Result<Vec<RcOption>, BazelrcError> {
    let base: Vec<RcOption> = rc.options_for(command).into_iter().cloned().collect();
    let expander = Expander {
        rc,
        command,
        skip_config_if_missing,
    };
    let mut result = Vec::new();
    expander.expand(&base, &mut Vec::new(), &mut result)?;

    if let Some(enabler) = platform_config_enabler(rc, &result)
        && !expander.config_options(PLATFORM_CONFIG).is_empty()
    {
        let requester = result[enabler].clone();
        let mut platform = Vec::new();
        expander.expand_config(PLATFORM_CONFIG, &requester, &mut Vec::new(), &mut platform)?;
        result.splice(enabler + 1..enabler + 1, platform);
    }
    Ok(result)
}

/// The index of the last `--enable_platform_specific_config` setting among
/// `opts` that applies at `rc`'s Bazel version, when that setting turns it on.
/// Bazel's default is off.
fn platform_config_enabler(rc: &Bazelrc, opts: &[RcOption]) -> Option<usize> {
    let (index, enabled) = opts
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, opt)| rc.version_condition_holds(&opt.version_condition))
        .find_map(|(i, opt)| Some((i, bool_setting(&opt.value, PLATFORM_CONFIG_OPTION)?)))?;
    enabled.then_some(index)
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
            self.expand_config(config_name, &args[index], ancestor_chain, result)?;
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
    /// [`expand_configs`]).
    fn expand_config(
        &self,
        config_name: &str,
        requester: &RcOption,
        ancestor_chain: &mut Vec<String>,
        result: &mut Vec<RcOption>,
    ) -> Result<(), BazelrcError> {
        if ancestor_chain.iter().any(|seen| seen == config_name) {
            let mut cycle = ancestor_chain.clone();
            cycle.push(config_name.to_owned());
            return Err(BazelrcError::ConfigCycle { cycle });
        }

        let mut config_opts = self.config_options(config_name);
        if config_opts.is_empty() {
            if self.skip_config_if_missing.contains(&config_name) {
                return Ok(());
            }
            return Err(BazelrcError::UndefinedConfig {
                command: self.command.to_owned(),
                name: config_name.to_owned(),
            });
        }

        for opt in &mut config_opts {
            if opt.version_condition.is_none() {
                opt.version_condition = requester.version_condition.clone();
            }
            if opt.section() != "common" {
                opt.command = requester.command.clone();
                opt.source_index = requester.source_index;
            }
        }

        ancestor_chain.push(config_name.to_owned());
        let expanded = self.expand(&config_opts, ancestor_chain, result);
        ancestor_chain.pop();
        expanded
    }

    /// The options `config_name` defines for the command, as written: every
    /// section the command reads contributes its `{section}:{config}` block, so
    /// `build:opt` and `test:opt` both apply to `bazel test --config=opt`.
    fn config_options(&self, config_name: &str) -> Vec<RcOption> {
        super::sections_for(self.command)
            .into_iter()
            .flat_map(|prefix| self.rc.raw_options(&format!("{prefix}:{config_name}")))
            .cloned()
            .collect()
    }
}
