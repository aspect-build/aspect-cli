mod discover;
mod expand;
mod parse;
pub(crate) mod preprocess;
pub(crate) mod tokenize;

/// Returns ancestor commands for `command` in Bazel's inheritance hierarchy,
/// ordered from most general to most specific (excluding `common` / `always`).
///
/// Mirrors Bazel's own command graph:
/// - `test`, `run`, `clean`, `mobile-install`, `info`, `print_action`, `config`, `cquery`, `aquery` → `build`
/// - `coverage`, `fetch`, `vendor` → `build`, `test`
pub(crate) fn command_ancestors(command: &str) -> &'static [&'static str] {
    match command {
        "test" | "run" | "clean" | "mobile-install" | "info" | "print_action" | "config"
        | "cquery" | "aquery" => &["build"],
        "coverage" | "fetch" | "vendor" => &["build", "test"],
        _ => &[],
    }
}

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use std::fmt;

use allocative::Allocative;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values::none::NoneOr;
use starlark::values::{
    NoSerialize, ProvidesStaticType, StarlarkValue, UnpackValue, Value, starlark_value,
    type_repr::StarlarkTypeRepr,
};
use thiserror::Error;

/// A single option value parsed from a bazelrc file.
#[derive(Debug, Clone, Default)]
pub struct RcOption {
    /// The option string, e.g. `"--jobs=4"`.
    pub value: String,
    /// The command section this option came from, e.g. `"build"`, `"build:opt"`,
    /// `"common"`, or `"always"`.
    pub command: String,
    /// Index into `BazelRC::sources` identifying which file this came from.
    pub source_index: usize,
    /// Set to `Some(cond)` when the option originates from a
    /// `try-import-if-bazel-version <cond> <path>` block.  The condition is an
    /// opaque string (e.g. `">=8.0.0"`, `"~8"`); version evaluation is left to
    /// the caller.
    pub version_condition: Option<String>,
}

impl fmt::Display for RcOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.command, self.value)?;
        if let Some(cond) = &self.version_condition {
            write!(f, " [if {cond}]")?;
        }
        Ok(())
    }
}

/// Errors that can occur when loading or using a `BazelRC`.
#[derive(Debug, Error)]
pub enum BazelRcError {
    #[error("import target not found: {path}")]
    ImportNotFound { path: PathBuf },

    #[error("bazelrc file not found: {path}")]
    BazelrcNotFound { path: PathBuf },

    #[error("import cycle detected: {}", chain.join(" → "))]
    ImportCycle { chain: Vec<String> },

    #[error("--config expansion cycle: {}", cycle.join(" → "))]
    ConfigCycle { cycle: Vec<String> },

    #[error("undefined config '{name}' for command '{command}'")]
    UndefinedConfig { command: String, name: String },

    #[error("invalid import directive arguments: {directive}")]
    InvalidImportArgs { directive: String },

    #[error("I/O error reading {file}: {source}")]
    Io {
        file: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Parsed representation of one or more `.bazelrc` files, plus the user-supplied
/// startup flags, `--config` skip policy, and Bazel version context that make it
/// the complete resolved set of run commands for invoking Bazel.
///
/// This is the Starlark `bazel.RunCommand` value — constructed by
/// `ctx.bazel.parse_rc()` / `ctx.bazel.new_rc()`, composed with `merge`,
/// inspected with `flag_value` / `bool_flag` / `flag_values`, and handed to
/// `ctx.bazel.build` / `test` / `query`.
#[derive(Debug, Clone, Default, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BazelRC {
    /// Map from command key (e.g. `"build"`, `"build:opt"`) to its options.
    #[allocative(skip)]
    options: HashMap<String, Vec<RcOption>>,
    /// Ordered list of source files that were loaded.
    #[allocative(skip)]
    sources: Vec<PathBuf>,
    /// `--config` names to drop when undefined (so a sidecar `query` does not
    /// fail on a config that only the build command defines).
    #[allocative(skip)]
    skip_config_if_missing: Vec<String>,
    /// User-provided startup flags (e.g. `--output_base=`), distinct from the
    /// rc-file `startup` section; a Bazel invocation sources startup options
    /// from here.
    #[allocative(skip)]
    startup_flags: Vec<String>,
    /// Bazel version used to evaluate version-gated options; `None` falls back
    /// to assuming the latest. Resolved at construction.
    #[allocative(skip)]
    version: Option<semver::Version>,
}

starlark_simple_value!(BazelRC);

#[starlark_value(type = "bazel.RunCommand")]
impl<'v> StarlarkValue<'v> for BazelRC {
    fn get_methods() -> Option<&'static starlark::environment::Methods> {
        static RES: starlark::environment::MethodsStatic =
            starlark::environment::MethodsStatic::new();
        RES.methods(runcommand_methods)
    }
}

impl fmt::Display for BazelRC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BazelRC(sources: [")?;
        for (i, src) in self.sources.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", src.display())?;
        }
        write!(f, "], keys: {})", self.options.len())
    }
}

impl BazelRC {
    /// Load and parse all applicable bazelrc files.
    ///
    /// - `root`: workspace root directory (used for `%workspace%` substitution and workspace rc).
    /// - `startup_flags`: startup flags that may contain `--bazelrc=`, `--nosystem_rc`, etc.
    ///   These are Bazel startup options (before the command), not command flags.
    /// - `flags`: non-startup command-line flags (e.g. `--config=foo`). These are stored as
    ///   `always` options so they are included in every `options_for` / `expand_configs` call.
    pub fn new<S1: AsRef<str>>(
        root: impl AsRef<Path>,
        startup_flags: &[S1],
        flags: &[RcOption],
    ) -> Result<Self, BazelRcError> {
        let root = root.as_ref();
        let rc_files = discover::discover_rc_files(root, startup_flags)?;

        let mut sources: Vec<PathBuf> = Vec::new();
        let mut options: HashMap<String, Vec<RcOption>> = HashMap::new();
        let mut import_stack: Vec<PathBuf> = Vec::new();

        for path in rc_files {
            parse::parse_file(
                &path,
                root,
                None,
                &mut sources,
                &mut options,
                &mut import_stack,
            )?;
        }

        // Append caller-supplied flags as synthetic `always` options so they participate in
        // options_for() and expand_configs() like any rc-file entry.
        if !flags.is_empty() {
            let cli_source_index = sources.len();
            sources.push(PathBuf::from("<command line>"));
            let always_opts = options.entry("always".to_owned()).or_default();
            for flag in flags {
                always_opts.push(RcOption {
                    source_index: cli_source_index,
                    command: "always".to_owned(),
                    ..flag.clone()
                });
            }
        }

        Ok(BazelRC {
            options,
            startup_flags: startup_flags
                .iter()
                .map(|s| s.as_ref().to_string())
                .collect(),
            sources,
            ..Default::default()
        })
    }

    /// Carry the `--config` skip policy (consumed by query/inspection so an
    /// undefined config doesn't error). Builder; returns `self`.
    pub fn with_skip_config_if_missing(mut self, skip: Vec<String>) -> Self {
        self.skip_config_if_missing = skip;
        self
    }

    /// Carry user-provided startup flags. Builder; returns `self`.
    pub fn with_startup_flags(mut self, startup_flags: Vec<String>) -> Self {
        self.startup_flags = startup_flags;
        self
    }

    /// Set the Bazel version used to evaluate version-gated options. Builder.
    pub fn with_version(mut self, version: Option<semver::Version>) -> Self {
        self.version = version;
        self
    }

    /// The user-provided startup flags carried by this run command.
    pub fn startup_flag_values(&self) -> &[String] {
        &self.startup_flags
    }

    /// Whether any option (in any section) is version-gated — lets a caller
    /// skip probing the Bazel version when no gate would consult it.
    pub fn has_version_gated_options(&self) -> bool {
        self.options
            .values()
            .flatten()
            .any(|o| o.version_condition.is_some())
    }

    /// A `BazelRC` that reads no files — only the given command `flags`,
    /// stored as synthetic `always` options. Used by `ctx.bazel.new_rc()` to
    /// build a run command from scratch with no `.bazelrc` on disk involved.
    pub fn blank(flags: &[RcOption]) -> Self {
        let mut options: HashMap<String, Vec<RcOption>> = HashMap::new();
        let mut sources: Vec<PathBuf> = Vec::new();
        if !flags.is_empty() {
            let cli_source_index = 0;
            sources.push(PathBuf::from("<command line>"));
            let always_opts = options.entry("always".to_owned()).or_default();
            for flag in flags {
                always_opts.push(RcOption {
                    source_index: cli_source_index,
                    command: "always".to_owned(),
                    ..flag.clone()
                });
            }
        }
        BazelRC {
            options,
            sources,
            ..Default::default()
        }
    }

    /// The ordered list of source files that were loaded.
    pub fn sources(&self) -> &[PathBuf] {
        &self.sources
    }

    /// Every option string across all command sections, in no particular
    /// order. Used to seed redaction allowlists (e.g.
    /// `--build_metadata=ALLOW_ENV=…`) before rendering the announce output.
    pub fn all_option_values(&self) -> impl Iterator<Item = &str> {
        self.options
            .values()
            .flat_map(|opts| opts.iter().map(|o| o.value.as_str()))
    }

    /// Return the source file path for the given option.
    pub fn source_of(&self, option: &RcOption) -> &Path {
        &self.sources[option.source_index]
    }

    /// Direct map lookup — returns options for exactly this key (e.g. `"build"` or `"build:opt"`).
    pub fn raw_options(&self, command_key: &str) -> &[RcOption] {
        self.options
            .get(command_key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Return all options applicable to `command`, respecting Bazel's command inheritance.
    ///
    /// Order: RC-file `always` + `common` + ancestor commands (general → specific) +
    /// `<command>` + CLI-provided flags.
    ///
    /// CLI-provided flags (those passed via the `flags` parameter to `BazelRC::new`, stored as
    /// `always` with source `"<command line>"`) are placed **last** so that any `--config=`
    /// flags they carry expand after all RC-file flags.  This matches Bazel's own semantics
    /// where command-line flags override `.bazelrc` defaults under last-write-wins.
    ///
    /// For example, `options_for("test")` returns:
    ///   `always` (rc-file) + `common` + `build` + `test` + `always` (cli)
    pub fn options_for(&self, command: &str) -> Vec<&RcOption> {
        // CLI-provided flags live in the "always" bucket but with source "<command line>".
        // We separate them so they can be appended last. A merged BazelRC can carry more than
        // one "<command line>" source (one per merged input), so match on the path rather than a
        // single index; the "always" bucket preserves insertion order, so a merged overlay's CLI
        // flags follow the base's and win under last-write-wins.
        let is_cli = |o: &RcOption| {
            self.sources
                .get(o.source_index)
                .is_some_and(|p| p.as_path() == std::path::Path::new("<command line>"))
        };

        let mut result = Vec::new();
        // RC-file always flags first (not from the synthetic CLI source)
        if let Some(opts) = self.options.get("always") {
            result.extend(opts.iter().filter(|o| !is_cli(o)));
        }
        // common, then ancestor commands, then the command itself
        if let Some(opts) = self.options.get("common") {
            result.extend(opts.iter());
        }
        for ancestor in command_ancestors(command) {
            if let Some(opts) = self.options.get(*ancestor) {
                result.extend(opts.iter());
            }
        }
        if let Some(opts) = self.options.get(command) {
            result.extend(opts.iter());
        }
        // CLI-provided flags last — they must override all RC-file flags
        if let Some(opts) = self.options.get("always") {
            result.extend(opts.iter().filter(|o| is_cli(o)));
        }
        result
    }

    /// Render a human-readable summary of all options loaded for `command`,
    /// for the `--announce-bazel-rc` disclosure.
    ///
    /// Output is grouped by source file, with flags wrapped at `max_width`
    /// columns; when `ansi` is true, headers and section names are styled with
    /// ANSI escapes. Version-gated flags (from `try-import-if-bazel-version`)
    /// are annotated with `[if <cond>]`.
    ///
    /// `root` is the workspace root and `home` the user's home directory; they
    /// drive how source paths are displayed (see `shorten`). `redact` scrubs
    /// each flag's value before it's printed — the caller passes the same
    /// redaction the rest of the CLI uses, so secrets in `--remote_header=…` /
    /// `--action_env=…` etc. don't leak into CI logs.
    pub fn announce(
        &self,
        command: &str,
        ansi: bool,
        max_width: usize,
        root: &Path,
        home: Option<&Path>,
        redact: impl Fn(&str) -> String,
    ) -> String {
        // `d` uses 256-color grey rather than SGR 2 (faint) because GitHub
        // Actions' log viewer silently drops SGR 2 — the section keys would
        // render at full weight on GHA even though the gate fired. Matches
        // the styling aspect-cli's runtime announce lines and `tools/bazel`
        // already use for grey output.
        let (b, d, y, r) = if ansi {
            ("\x1b[1m", "\x1b[38;5;244m", "\x1b[33m", "\x1b[0m")
        } else {
            ("", "", "", "")
        };

        let fmt_flag = |opt: &RcOption| -> String {
            let value = redact(&opt.value);
            match &opt.version_condition {
                None => value,
                Some(cond) => format!("{}[if {}]{} {}", y, cond, r, value),
            }
        };

        // Display an rc source path relative to the most meaningful anchor:
        //   - under the workspace root → `./relative/path`
        //   - else under $HOME         → `~/relative/path`
        //   - else                     → the absolute path
        // so the reader can tell workspace, user, and system rc files apart.
        let shorten = |p: &Path| -> String {
            if p == Path::new("<command line>") {
                return "client".to_owned();
            }
            if let Ok(rel) = p.strip_prefix(root) {
                return format!("./{}", rel.display());
            }
            if let Some(home) = home
                && let Ok(rel) = p.strip_prefix(home)
            {
                return format!("~/{}", rel.display());
            }
            p.display().to_string()
        };

        // Fit flags onto lines starting at `start_col`, wrapping at `max_width`.
        // Continuation lines are padded with `cont_indent` spaces.
        let wrap_flags = |flags: &[String], start_col: usize, cont_indent: usize| -> String {
            if flags.is_empty() {
                return String::new();
            }
            let indent = " ".repeat(cont_indent);
            let mut result = String::new();
            let mut col = start_col;
            let mut first = true;
            for flag in flags {
                let flag_len = flag.len();
                if first {
                    result.push_str(flag);
                    col += flag_len;
                    first = false;
                } else if col + 1 + flag_len <= max_width {
                    result.push(' ');
                    result.push_str(flag);
                    col += 1 + flag_len;
                } else {
                    result.push('\n');
                    result.push_str(&indent);
                    result.push_str(flag);
                    col = cont_indent + flag_len;
                }
            }
            result
        };

        // Collect all config keys relevant to this command. Ancestor bases
        // count too (e.g. `build:ci` applies to `test --config=ci`), matching
        // the direct-section inheritance below and what Bazel applies.
        //
        // Base expansion order, per `expand::expand_args`:
        //   always → common → ancestors(general→specific) → command
        // `base_rank` maps each base to that position so two sections setting
        // the same flag (e.g. `common:ci` then `build:ci`) are announced in the
        // override order Bazel actually applies — not alphabetically, which
        // would invert `build:ci` ahead of `common:ci`. Keys are then grouped
        // by config name (alphabetical) for deterministic output.
        let ancestors = command_ancestors(command);
        let base_rank = |base: &str| -> usize {
            match base {
                "always" => 0,
                "common" => 1,
                _ if base == command => 2 + ancestors.len(),
                _ => 2 + ancestors.iter().position(|a| *a == base).unwrap_or(0),
            }
        };
        let mut config_keys: Vec<&String> = self
            .options
            .keys()
            .filter(|k| {
                if let Some(base) = k.split(':').next() {
                    k.contains(':')
                        && (base == "always"
                            || base == "common"
                            || base == command
                            || ancestors.contains(&base))
                } else {
                    false
                }
            })
            .collect();
        config_keys.sort_by(|a, b| {
            let (a_base, a_cfg) = a.split_once(':').unwrap_or(("", a.as_str()));
            let (b_base, b_cfg) = b.split_once(':').unwrap_or(("", b.as_str()));
            a_cfg
                .cmp(b_cfg)
                .then(base_rank(a_base).cmp(&base_rank(b_base)))
        });

        let mut out = String::new();
        let mut first_block = true;

        // Single pass per source file: emit direct sections then config sections together.
        //
        // Include the command's ancestors (e.g. `build` for `test`) so the
        // announce reflects what Bazel actually applies — `options_for` already
        // pulls ancestor flags into the spawn via `command_ancestors`, and
        // omitting them here made the announce understate the effective config
        // (e.g. a `build --disk_cache=…` flag missing from a `test` announce).
        let mut direct_keys = vec!["startup", "always", "common"];
        direct_keys.extend(command_ancestors(command));
        direct_keys.push(command);
        for (source_idx, source_path) in self.sources.iter().enumerate() {
            let is_client = source_path == Path::new("<command line>");
            let short = shorten(source_path);

            // Collect direct sections for this source.
            let direct_sections: Vec<(&str, Vec<String>)> = direct_keys
                .iter()
                .filter_map(|&key| {
                    let flags: Vec<String> = self
                        .options
                        .get(key)
                        .map(|v| {
                            v.iter()
                                .filter(|o| o.source_index == source_idx)
                                .map(|o| fmt_flag(o))
                                .collect()
                        })
                        .unwrap_or_default();
                    if flags.is_empty() {
                        None
                    } else {
                        Some((key, flags))
                    }
                })
                .collect();

            // Collect config sections for this source.
            let config_sections: Vec<(&str, Vec<String>)> = config_keys
                .iter()
                .filter_map(|&key| {
                    let flags: Vec<String> = self
                        .options
                        .get(key)
                        .map(|v| {
                            v.iter()
                                .filter(|o| o.source_index == source_idx)
                                .map(|o| fmt_flag(o))
                                .collect()
                        })
                        .unwrap_or_default();
                    if flags.is_empty() {
                        None
                    } else {
                        Some((key.as_str(), flags))
                    }
                })
                .collect();

            if direct_sections.is_empty() && config_sections.is_empty() {
                continue;
            }

            if !first_block {
                out.push('\n');
            }
            first_block = false;

            if is_client {
                // Client flags: flatten all direct sections inline on one labelled line.
                let all_flags: Vec<String> = direct_sections
                    .into_iter()
                    .flat_map(|(_, flags)| flags)
                    .collect();
                let prefix_plain_len = short.len() + 2;
                let wrapped = wrap_flags(&all_flags, prefix_plain_len, prefix_plain_len);
                out.push_str(&format!("{}{}{}  {}\n", b, short, r, wrapped));
            } else {
                out.push_str(&format!("{}{}{}\n", b, short, r));

                // Combine direct and config section names to find max label width for alignment.
                let all_sections: Vec<(&str, Vec<String>)> = direct_sections
                    .into_iter()
                    .chain(config_sections.into_iter())
                    .collect();
                let max_key_len = all_sections.iter().map(|(k, _)| k.len()).max().unwrap_or(0);

                for (key, flags) in &all_sections {
                    let padding = " ".repeat(max_key_len - key.len());
                    let prefix_plain_len = 2 + key.len() + padding.len() + 2;
                    let wrapped = wrap_flags(flags, prefix_plain_len, prefix_plain_len);
                    out.push_str(&format!("  {}{}{}{}{}\n", d, key, r, padding, {
                        if wrapped.is_empty() {
                            String::new()
                        } else {
                            format!("  {}", wrapped)
                        }
                    }));
                }
            }
        }

        out
    }

    /// Append a synthetic `always` option, optionally version-gated.
    ///
    /// The option is attributed to the synthetic `<command line>` source (created on first use).
    pub fn push_flag(&mut self, value: &str, version_condition: Option<&str>) {
        let source_index = self
            .sources
            .iter()
            .position(|p| p == std::path::Path::new("<command line>"))
            .unwrap_or_else(|| {
                let idx = self.sources.len();
                self.sources.push(PathBuf::from("<command line>"));
                idx
            });
        self.options
            .entry("always".to_owned())
            .or_default()
            .push(RcOption {
                value: value.to_owned(),
                command: "always".to_owned(),
                source_index,
                version_condition: version_condition.map(|s| s.to_owned()),
            });
    }

    /// Overlay `other` on top of `self`, returning a new `BazelRC`.
    ///
    /// `other` wins on conflicts: within each command section its options are
    /// appended **after** `self`'s, so Bazel's last-write-wins ordering resolves
    /// in `other`'s favor. Sources are concatenated, with `other`'s
    /// `source_index` values rebased onto the combined list. Neither input is
    /// mutated.
    pub fn merge(&self, other: &BazelRC) -> BazelRC {
        let mut merged = self.clone();
        let offset = merged.sources.len();
        merged.sources.extend(other.sources.iter().cloned());
        for (command, opts) in &other.options {
            let bucket = merged.options.entry(command.clone()).or_default();
            for opt in opts {
                let mut opt = opt.clone();
                opt.source_index += offset;
                bucket.push(opt);
            }
        }
        merged
            .startup_flags
            .extend(other.startup_flags.iter().cloned());
        for s in &other.skip_config_if_missing {
            if !merged.skip_config_if_missing.contains(s) {
                merged.skip_config_if_missing.push(s.clone());
            }
        }
        if other.version.is_some() {
            merged.version = other.version.clone();
        }
        merged
    }

    /// Expand all `--config=` flags for the given command.
    ///
    /// Starts from `options_for(command)` (which includes `always`, `common`, and command-specific
    /// options, plus any CLI flags passed to `BazelRC::new`). Returns the fully-expanded flat list
    /// of `RcOption`s with all `--config=` references resolved.
    ///
    /// Each entry's `version_condition` is `Some(cond)` when the option originated from a
    /// `try-import-if-bazel-version` block; `None` otherwise. Callers can inspect this field to
    /// decide how to handle version-gated flags.
    pub fn expand_configs(
        &self,
        command: &str,
        skip_config_if_missing: &[&str],
    ) -> Result<Vec<RcOption>, BazelRcError> {
        expand::expand_configs(self, command, skip_config_if_missing)
    }

    /// Assemble the `(startup_flags, command_flags)` to run `command` with this
    /// run command. `command_flags` are the config-expanded, version-filtered
    /// options (common-section ones rendered as `--default_override=0:common=…`);
    /// `startup_flags` are the user startup flags plus the rc-file `startup`
    /// section plus `--ignore_all_rc_files` (the on-disk rc is already absorbed).
    pub fn resolve_for_command(&self, command: &str) -> anyhow::Result<(Vec<String>, Vec<String>)> {
        let opts = self.applicable_options(command, None)?;
        let (overrides, regular) = partition_expand_all(&opts);
        let command_flags: Vec<String> = overrides
            .into_iter()
            .map(|(v, _)| v)
            .chain(regular.into_iter().map(|(v, _)| v))
            .collect();
        Ok((self.invocation_startup_flags(), command_flags))
    }

    /// The startup flags Bazel runs with when this run command is in effect:
    /// the user startup flags, the rc-file `startup` section, and
    /// `--ignore_all_rc_files` (the on-disk rc is already absorbed). Command-
    /// independent; the single source for build/test/query/info/health_check.
    pub fn invocation_startup_flags(&self) -> Vec<String> {
        let mut startup = self.startup_flags.clone();
        // Gate the rc-file `startup` section by version like command options: a
        // `try-import-if-bazel-version` startup flag meant for another Bazel
        // version must not be passed to the running one.
        startup.extend(
            self.raw_options("startup")
                .iter()
                .filter(|o| self.version_condition_holds(&o.version_condition))
                .map(|o| o.value.clone()),
        );
        startup.push("--ignore_all_rc_files".to_string());
        startup
    }

    /// Whether a version-gated option's `condition` holds at this run command's
    /// configured Bazel version (assumed-latest when unset). `None` → always.
    /// A malformed constraint is treated as applicable; the build surfaces it.
    fn version_condition_holds(&self, condition: &Option<String>) -> bool {
        let Some(cond) = condition else {
            return true;
        };
        let assumed_latest = semver::Version::new(u64::MAX, u64::MAX, u64::MAX);
        let probe = self.version.as_ref().unwrap_or(&assumed_latest);
        semver::VersionReq::parse(cond)
            .map(|req| req.matches(probe))
            .unwrap_or(true)
    }

    /// Config-expanded options for `command`, filtered to those that apply at
    /// the effective Bazel version (the per-call `version_override`, else the
    /// configured `version`, else assumed-latest). Used by the value queries.
    fn applicable_options(
        &self,
        command: &str,
        version_override: Option<&str>,
    ) -> anyhow::Result<Vec<RcOption>> {
        let skip: Vec<&str> = self
            .skip_config_if_missing
            .iter()
            .map(String::as_str)
            .collect();
        let opts = self
            .expand_configs(command, &skip)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        if !opts.iter().any(|o| o.version_condition.is_some()) {
            return Ok(opts);
        }

        let version = match version_override {
            Some(v) => semver::Version::parse(v).ok(),
            None => self.version.clone(),
        };
        let assumed_latest = semver::Version::new(u64::MAX, u64::MAX, u64::MAX);
        let probe = version.as_ref().unwrap_or(&assumed_latest);

        let mut out = Vec::with_capacity(opts.len());
        for opt in opts {
            match &opt.version_condition {
                None => out.push(opt),
                Some(cond) => {
                    let req = semver::VersionReq::parse(cond).map_err(|e| {
                        anyhow::anyhow!("invalid version constraint '{}': {}", cond, e)
                    })?;
                    if req.matches(probe) {
                        out.push(opt);
                    }
                }
            }
        }
        Ok(out)
    }
}

/// Split an expanded option list into `(default_override_flags, regular_flags)`.
///
/// `common`-section options become `--default_override=0:common=<value>` strings, returned first
/// so they precede user flags and lose to them under last-write-wins. Each entry keeps its
/// `version_condition` so version-gated options aren't silently promoted to unconditional.
///
/// A two-token option (`--flag value`, parsed as a bare-flag `RcOption` followed by its value
/// `RcOption`) is folded into the canonical `--flag=value` form for its single
/// `--default_override=0:common=--flag=value` entry. Bazel parses each override value as one
/// option token and does NOT re-split it on whitespace, so neither two separate overrides (the
/// value strands as its own unrecognized option) nor a space-joined `--flag value` override
/// (rejected as a single unrecognized option) works. Regular flags need no such fold — they
/// pass through as adjacent argv tokens, which Bazel pairs natively.
///
/// Accepted trade-off: a bare token following a boolean flag is folded as if it were the flag's
/// value, even when it is really a non-option argument. Bazel's bazelrc supports non-option args
/// in rc lines (e.g. `common --keep_going //:all`, where `//:all` is a default target prepended
/// to the command line); under this fold that becomes `--keep_going=//:all` and the target is
/// lost. Telling the two apart needs a per-flag "takes a value" registry we don't have, and the
/// value-flag case (`--remote_download_outputs minimal`) is by far the common one — so we do not
/// support non-option arguments in `common`-section rc lines. Put default targets on the command
/// line or in a `build`-section (non-`common`) rc line, which passes through untouched.
fn partition_expand_all(
    opts: &[RcOption],
) -> (Vec<(String, Option<String>)>, Vec<(String, Option<String>)>) {
    let mut overrides = Vec::new();
    let mut flags = Vec::new();
    let mut i = 0;
    while i < opts.len() {
        let opt = &opts[i];
        let base = opt.command.split(':').next().unwrap_or(&opt.command);
        if base == "common" {
            let mut value = opt.value.clone();
            if let Some(next) = opts.get(i + 1) {
                if is_two_token_pair(opt, next) {
                    value = format!("{}={}", opt.value, next.value);
                    i += 1;
                }
            }
            overrides.push((
                format!("--default_override=0:common={value}"),
                opt.version_condition.clone(),
            ));
        } else {
            flags.push((opt.value.clone(), opt.version_condition.clone()));
        }
        i += 1;
    }
    (overrides, flags)
}

/// Whether `flag` is a bare option whose value is the immediately following `value` option —
/// the two-token `--flag value` form. Mirrors `flag_value_list`'s next-token pairing: `flag`
/// must start with `-` and carry no `=`, `value` must not start with `-`, and both must share
/// the same command section and version condition (i.e. originate from the same rc line).
///
/// Purely positional — it cannot tell a value-taking flag from a boolean flag trailed by a
/// non-option argument, so `--keep_going //:all` also matches. See `partition_expand_all` for
/// why that trade-off is accepted (non-option args in `common` rc lines are unsupported).
fn is_two_token_pair(flag: &RcOption, value: &RcOption) -> bool {
    flag.value.starts_with('-')
        && !flag.value.contains('=')
        && !value.value.starts_with('-')
        && flag.command == value.command
        && flag.version_condition == value.version_condition
}

/// All values of a repeatable `--name` option among `opts`, in order. Matches
/// `--name=VALUE` and the two-token `--name VALUE` form (value is the next token).
fn flag_value_list(opts: &[RcOption], name: &str) -> Vec<String> {
    let eq_prefix = format!("{name}=");
    let mut values = Vec::new();
    for (i, opt) in opts.iter().enumerate() {
        let v = opt.value.as_str();
        if let Some(rest) = v.strip_prefix(&eq_prefix) {
            values.push(rest.to_string());
        } else if v == name {
            if let Some(next) = opts.get(i + 1) {
                if !next.value.starts_with('-') {
                    values.push(next.value.clone());
                }
            }
        }
    }
    values
}

/// Tri-state of a boolean `--name` option: `Some(true)` for `--name` /
/// `--name=true|1|yes`, `Some(false)` for `--noname` / `--name=false|0|no`,
/// `None` if unset. Last occurrence wins.
fn last_bool_flag(opts: &[RcOption], name: &str) -> Option<bool> {
    let bare = name.strip_prefix("--").unwrap_or(name);
    let negated = format!("--no{bare}");
    let eq_prefix = format!("{name}=");
    let mut state = None;
    for opt in opts {
        let v = opt.value.as_str();
        if v == name {
            state = Some(true);
        } else if v == negated {
            state = Some(false);
        } else if let Some(rest) = v.strip_prefix(&eq_prefix) {
            match rest {
                "true" | "1" | "yes" => state = Some(true),
                "false" | "0" | "no" => state = Some(false),
                _ => {}
            }
        }
    }
    state
}

/// `RcOption` unpacks from a plain `str` or a `(flag, version_constraint)` tuple,
/// so `parse_rc` / `new_rc` accept the same flag shape as `ctx.bazel.build`.
impl StarlarkTypeRepr for RcOption {
    type Canonical = <starlark::values::tuple::UnpackTuple<String> as StarlarkTypeRepr>::Canonical;
    fn starlark_type_repr() -> starlark::typing::Ty {
        // `str | (str, str)` — represented loosely as a string-or-tuple input.
        starlark::values::tuple::UnpackTuple::<String>::starlark_type_repr()
    }
}

impl<'v> UnpackValue<'v> for RcOption {
    type Error = anyhow::Error;

    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        if let Some(s) = value.unpack_str() {
            return Ok(Some(RcOption {
                value: s.to_owned(),
                ..RcOption::default()
            }));
        }
        if let Some(tup) = starlark::values::tuple::UnpackTuple::<String>::unpack_value_impl(value)?
        {
            let mut items = tup.items.into_iter();
            if let Some(flag) = items.next() {
                return Ok(Some(RcOption {
                    value: flag,
                    version_condition: items.next(),
                    ..RcOption::default()
                }));
            }
        }
        Ok(None)
    }
}

fn alloc_flag<'v>(eval: &mut Evaluator<'v, '_, '_>, opt: &RcOption) -> Value<'v> {
    match &opt.version_condition {
        None => eval.heap().alloc(opt.value.as_str()),
        Some(cond) => eval.heap().alloc((opt.value.as_str(), cond.as_str())),
    }
}

#[starlark::starlark_module]
fn runcommand_methods(registry: &mut starlark::environment::MethodsBuilder) {
    /// Overlay `other` on top of this run command, returning a new one. `other`
    /// wins on conflicts (its options apply after this one's under last-write-wins);
    /// startup flags, `--config` skip policy, and `version` are merged with `other`
    /// taking precedence when set. Neither input is mutated.
    fn merge<'v>(
        this: &BazelRC,
        #[starlark(require = pos)] other: &BazelRC,
    ) -> anyhow::Result<BazelRC> {
        Ok(this.merge(other))
    }

    /// The effective value of `--name` for `command`, or `None` if unset.
    /// Last-write-wins over the config-expanded options; matches `--name=VALUE`
    /// and the two-token `--name VALUE` form. `version` overrides the Bazel
    /// version used for version-gated options.
    fn flag_value<'v>(
        this: &BazelRC,
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] command: &str,
        #[starlark(require = named, default = NoneOr::None)] version: NoneOr<String>,
    ) -> anyhow::Result<NoneOr<String>> {
        let opts = this.applicable_options(command, version.into_option().as_deref())?;
        Ok(match flag_value_list(&opts, name).pop() {
            Some(v) => NoneOr::Other(v),
            None => NoneOr::None,
        })
    }

    /// The tri-state of boolean `--name` for `command`: `True` for `--name` /
    /// `--name=true`, `False` for `--noname` / `--name=false`, `None` if unset.
    /// The caller owns the default when `None` (e.g. `--enable_bzlmod` defaults
    /// on for Bazel 7+).
    fn bool_flag<'v>(
        this: &BazelRC,
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] command: &str,
        #[starlark(require = named, default = NoneOr::None)] version: NoneOr<String>,
    ) -> anyhow::Result<NoneOr<bool>> {
        let opts = this.applicable_options(command, version.into_option().as_deref())?;
        Ok(match last_bool_flag(&opts, name) {
            Some(b) => NoneOr::Other(b),
            None => NoneOr::None,
        })
    }

    /// All values of a repeatable `--name` option for `command`, in order — for
    /// flags that legitimately accumulate (`--aspects`, `--output_groups`, …).
    fn flag_values<'v>(
        this: &BazelRC,
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] command: &str,
        #[starlark(require = named, default = NoneOr::None)] version: NoneOr<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        let opts = this.applicable_options(command, version.into_option().as_deref())?;
        Ok(flag_value_list(&opts, name)
            .into_iter()
            .map(|v| eval.heap().alloc(v))
            .collect())
    }

    /// Options applicable to `command` without expanding `--config=`. Each item
    /// is a `str` or a `(flag, version_condition)` tuple.
    fn options_for<'v>(
        this: &BazelRC,
        #[starlark(require = named)] command: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        Ok(this
            .options_for(&command)
            .into_iter()
            .map(|opt| alloc_flag(eval, opt))
            .collect())
    }

    /// Expand all `--config=` flags for `command` into the fully-resolved flag
    /// list, compatible with `ctx.bazel.build(flags=...)`.
    ///
    /// Version-gated options are filtered to the run command's resolved Bazel
    /// version (same as `flag_value` / `resolve_for_command`), so the result
    /// reflects what Bazel actually receives — an option gated to a different
    /// version is dropped rather than reported as active.
    fn expand<'v>(
        this: &BazelRC,
        #[starlark(require = named)] command: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        let opts = this.applicable_options(&command, None)?;
        Ok(opts.iter().map(|opt| alloc_flag(eval, opt)).collect())
    }

    /// Expand `--config=` flags for `command`, split into
    /// `(startup_flags, flags)` where `common`-section options are rendered as
    /// `--default_override=0:common=...` startup-side entries.
    ///
    /// Like `expand`, the command flags are filtered to the resolved Bazel
    /// version so version-gated options report accurately.
    fn expand_all<'v>(
        this: &BazelRC,
        #[starlark(require = named)] command: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<(Vec<Value<'v>>, Vec<Value<'v>>)> {
        let opts = this.applicable_options(&command, None)?;

        let startup: Vec<Value<'v>> = this
            .raw_options("startup")
            .iter()
            .map(|opt| alloc_flag(eval, opt))
            .collect();

        let (overrides, regular) = partition_expand_all(&opts);
        let flags: Vec<Value<'v>> = overrides
            .iter()
            .chain(regular.iter())
            .map(|(value, cond)| match cond {
                None => eval.heap().alloc(value.as_str()),
                Some(c) => eval.heap().alloc((value.as_str(), c.as_str())),
            })
            .collect();
        Ok((startup, flags))
    }

    /// The source file paths that were loaded.
    fn sources<'v>(
        this: &BazelRC,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        Ok(this
            .sources()
            .iter()
            .map(|p| eval.heap().alloc(p.display().to_string()))
            .collect())
    }

    /// The startup flags Bazel runs with under this run command: the user
    /// startup flags + the rc-file `startup` section + `--ignore_all_rc_files`.
    fn startup_flags<'v>(
        this: &BazelRC,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        Ok(this
            .invocation_startup_flags()
            .into_iter()
            .map(|s| eval.heap().alloc(s))
            .collect())
    }
}

#[cfg(test)]
mod runcommand_tests {
    use super::*;

    fn opts(values: &[&str]) -> Vec<RcOption> {
        values
            .iter()
            .map(|v| RcOption {
                value: v.to_string(),
                ..RcOption::default()
            })
            .collect()
    }

    #[test]
    fn flag_value_eq_form_last_wins() {
        let o = opts(&["--remote_cache=grpc://a", "--remote_cache=grpc://b"]);
        assert_eq!(
            flag_value_list(&o, "--remote_cache").pop().unwrap(),
            "grpc://b"
        );
    }

    #[test]
    fn flag_value_two_token_form() {
        let o = opts(&["--remote_cache", "grpc://x", "--jobs=4"]);
        assert_eq!(flag_value_list(&o, "--remote_cache"), vec!["grpc://x"]);
    }

    #[test]
    fn flag_value_bare_flag_has_no_value() {
        // A bare `--foo` followed by another flag yields no value.
        let o = opts(&["--remote_cache", "--jobs=4"]);
        assert!(flag_value_list(&o, "--remote_cache").is_empty());
    }

    #[test]
    fn flag_values_collects_all_repeats() {
        let o = opts(&["--aspects=//:a.bzl%a", "--aspects=//:b.bzl%b"]);
        assert_eq!(
            flag_value_list(&o, "--aspects"),
            vec!["//:a.bzl%a", "//:b.bzl%b"]
        );
    }

    #[test]
    fn bool_flag_positive_negative_and_unset() {
        assert_eq!(
            last_bool_flag(&opts(&["--enable_bzlmod"]), "--enable_bzlmod"),
            Some(true)
        );
        assert_eq!(
            last_bool_flag(&opts(&["--noenable_bzlmod"]), "--enable_bzlmod"),
            Some(false)
        );
        assert_eq!(
            last_bool_flag(&opts(&["--enable_bzlmod=false"]), "--enable_bzlmod"),
            Some(false)
        );
        assert_eq!(
            last_bool_flag(&opts(&["--jobs=4"]), "--enable_bzlmod"),
            None
        );
    }

    #[test]
    fn bool_flag_last_occurrence_wins() {
        let o = opts(&["--enable_bzlmod", "--noenable_bzlmod"]);
        assert_eq!(last_bool_flag(&o, "--enable_bzlmod"), Some(false));
    }

    #[test]
    fn partition_routes_common_to_overrides() {
        let mut common = RcOption {
            value: "--remote_cache=x".to_string(),
            command: "common".to_string(),
            ..RcOption::default()
        };
        let build = RcOption {
            value: "--jobs=4".to_string(),
            command: "build".to_string(),
            ..RcOption::default()
        };
        let (overrides, flags) = partition_expand_all(&[common.clone(), build]);
        assert_eq!(
            overrides,
            vec![(
                "--default_override=0:common=--remote_cache=x".to_string(),
                None
            )]
        );
        assert_eq!(flags, vec![("--jobs=4".to_string(), None)]);
        common.command = "common:ci".to_string();
        let (overrides, _) = partition_expand_all(&[common]);
        assert_eq!(overrides.len(), 1, "common:config base routes to overrides");
    }

    /// Build the two `RcOption`s a `common:ci --flag value` rc line parses into.
    fn common_ci(values: &[&str]) -> Vec<RcOption> {
        values
            .iter()
            .map(|v| RcOption {
                value: v.to_string(),
                command: "common:ci".to_string(),
                ..RcOption::default()
            })
            .collect()
    }

    #[test]
    fn partition_folds_two_token_common_override_to_eq_form() {
        // `common:ci --remote_download_outputs minimal` must become ONE override in
        // the canonical `=` form — Bazel does not re-split the override value on
        // whitespace, so two overrides (or a space-joined one) are both rejected.
        let (overrides, _) =
            partition_expand_all(&common_ci(&["--remote_download_outputs", "minimal"]));
        assert_eq!(
            overrides,
            vec![(
                "--default_override=0:common=--remote_download_outputs=minimal".to_string(),
                None
            )]
        );
    }

    #[test]
    fn partition_does_not_join_canonical_eq_common_override() {
        // `--flag=value` carries its own value; the next token is a separate flag.
        let (overrides, _) = partition_expand_all(&common_ci(&[
            "--remote_download_outputs=minimal",
            "--jobs=4",
        ]));
        assert_eq!(
            overrides,
            vec![
                (
                    "--default_override=0:common=--remote_download_outputs=minimal".to_string(),
                    None
                ),
                ("--default_override=0:common=--jobs=4".to_string(), None),
            ]
        );
    }

    #[test]
    fn partition_does_not_join_bool_flag_before_another_flag() {
        // A bare boolean flag followed by another flag is not a two-token pair.
        let (overrides, _) = partition_expand_all(&common_ci(&[
            "--remote_local_fallback",
            "--remote_timeout=3600",
        ]));
        assert_eq!(
            overrides,
            vec![
                (
                    "--default_override=0:common=--remote_local_fallback".to_string(),
                    None
                ),
                (
                    "--default_override=0:common=--remote_timeout=3600".to_string(),
                    None
                ),
            ]
        );
    }

    #[test]
    fn partition_folds_bool_flag_with_trailing_target_accepted_tradeoff() {
        // Documented limitation: a boolean flag trailed by a non-option target arg
        // is folded as `--flag=target`, losing the target. Non-option args in
        // `common` rc lines are unsupported (see `partition_expand_all`). Locks the
        // behavior in so a future "fix" is a conscious decision, not a surprise.
        let (overrides, _) = partition_expand_all(&common_ci(&["--keep_going", "//:all"]));
        assert_eq!(
            overrides,
            vec![(
                "--default_override=0:common=--keep_going=//:all".to_string(),
                None
            )]
        );
    }

    #[test]
    fn partition_does_not_join_across_version_condition() {
        // A bare flag and the next value must share a version condition to pair
        // (same rc line); otherwise they came from different lines.
        let flag = RcOption {
            value: "--remote_download_outputs".to_string(),
            command: "common:ci".to_string(),
            version_condition: Some(">=8".to_string()),
            ..RcOption::default()
        };
        let value = RcOption {
            value: "minimal".to_string(),
            command: "common:ci".to_string(),
            version_condition: None,
            ..RcOption::default()
        };
        let (overrides, _) = partition_expand_all(&[flag, value]);
        assert_eq!(
            overrides.len(),
            2,
            "differing version conditions must not join"
        );
    }

    #[test]
    fn merge_overlays_b_over_a_last_wins() {
        let a = BazelRC::blank(&opts(&["--remote_cache=grpc://a"]));
        let b = BazelRC::blank(&opts(&["--remote_cache=grpc://b"]));
        let merged = a.merge(&b);
        let resolved = merged.applicable_options("build", None).unwrap();
        assert_eq!(
            flag_value_list(&resolved, "--remote_cache").pop().unwrap(),
            "grpc://b"
        );
    }

    #[test]
    fn merge_combines_startup_and_version() {
        let a = BazelRC::blank(&[]).with_startup_flags(vec!["--output_base=/a".to_string()]);
        let b = BazelRC::blank(&[])
            .with_startup_flags(vec!["--host_jvm_args=-Xmx2g".to_string()])
            .with_version(Some(semver::Version::new(8, 0, 0)));
        let merged = a.merge(&b);
        assert_eq!(
            merged.startup_flag_values(),
            &["--output_base=/a", "--host_jvm_args=-Xmx2g"]
        );
        assert_eq!(merged.version, Some(semver::Version::new(8, 0, 0)));
    }

    #[test]
    fn version_gated_options_filtered_by_version() {
        let mut gated = RcOption {
            value: "--notmp_sandbox".to_string(),
            command: "always".to_string(),
            version_condition: Some(">=8".to_string()),
            ..RcOption::default()
        };
        let mut rc = BazelRC::blank(&[]);
        rc.options.insert("always".to_string(), vec![gated.clone()]);

        // On 7.x the >=8 gate drops it; on 8.x it applies.
        let rc7 = rc.clone().with_version(Some(semver::Version::new(7, 0, 0)));
        assert!(rc7.applicable_options("build", None).unwrap().is_empty());
        let rc8 = rc.clone().with_version(Some(semver::Version::new(8, 0, 0)));
        assert_eq!(rc8.applicable_options("build", None).unwrap().len(), 1);

        // Per-call override beats the stored version.
        gated.value = "--notmp_sandbox".to_string();
        assert!(
            rc8.applicable_options("build", Some("7.0.0"))
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Startup flags that prevent system/home rc files from being loaded, making tests hermetic.
    const ISOLATE: &[&str] = &["--nosystem_rc", "--nohome_rc"];

    fn make_workspace() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn flags(strs: &[&str]) -> Vec<RcOption> {
        strs.iter()
            .map(|s| RcOption {
                value: s.to_string(),
                ..RcOption::default()
            })
            .collect()
    }

    // ── Tokenizer edge cases ────────────────────────────────────────────────

    #[test]
    fn tokenizer_spec_table_comment_boundary() {
        assert_eq!(tokenize::tokenize("build # comment"), vec!["build"]);
    }

    #[test]
    fn tokenizer_spec_single_quoted_spaces() {
        assert_eq!(
            tokenize::tokenize("build '--test_env=FOO=a b'"),
            vec!["build", "--test_env=FOO=a b"]
        );
    }

    #[test]
    fn tokenizer_spec_double_quoted_spaces() {
        assert_eq!(
            tokenize::tokenize(r#"build "--test_env=FOO=a b""#),
            vec!["build", "--test_env=FOO=a b"]
        );
    }

    #[test]
    fn tokenizer_backslash_space() {
        assert_eq!(
            tokenize::tokenize(r"build --flag=a\ b"),
            vec!["build", "--flag=a b"]
        );
    }

    #[test]
    fn tokenizer_mixed_quotes() {
        // 'he'llo → hello (adjacent quote sections join into one token)
        assert_eq!(tokenize::tokenize("build 'he'llo"), vec!["build", "hello"]);
    }

    // ── Import cycle detection ───────────────────────────────────────────────

    #[test]
    fn import_cycle_detected() {
        let dir = make_workspace();
        let root = dir.path();

        let a = root.join("a.bazelrc");
        let b = root.join("b.bazelrc");
        fs::write(&a, format!("import {}\n", b.display())).unwrap();
        fs::write(&b, format!("import {}\n", a.display())).unwrap();

        let mut sources = Vec::new();
        let mut options = HashMap::new();
        let mut stack = Vec::new();
        let err =
            parse::parse_file(&a, root, None, &mut sources, &mut options, &mut stack).unwrap_err();
        assert!(matches!(err, BazelRcError::ImportCycle { .. }));
    }

    #[test]
    fn diamond_import_allowed() {
        let dir = make_workspace();
        let root = dir.path();

        let d = root.join("d.bazelrc");
        let b = root.join("b.bazelrc");
        let c = root.join("c.bazelrc");
        let a = root.join("a.bazelrc");

        fs::write(&d, "build --jobs=4\n").unwrap();
        fs::write(&b, format!("import {}\n", d.display())).unwrap();
        fs::write(&c, format!("import {}\n", d.display())).unwrap();
        fs::write(
            &a,
            format!(
                r#"import {}
import {}
"#,
                b.display(),
                c.display()
            ),
        )
        .unwrap();

        let mut sources = Vec::new();
        let mut options = HashMap::new();
        let mut stack = Vec::new();
        parse::parse_file(&a, root, None, &mut sources, &mut options, &mut stack).unwrap();

        // d is imported twice → --jobs=4 appears twice
        assert_eq!(options.get("build").map(|v| v.len()), Some(2));
    }

    // ── announce ─────────────────────────────────────────────────────────────

    fn rc_with(sources: &[&Path], options: &[(&str, usize, &str)]) -> BazelRC {
        let mut map: HashMap<String, Vec<RcOption>> = HashMap::new();
        for (key, source_index, value) in options {
            map.entry(key.to_string()).or_default().push(RcOption {
                value: value.to_string(),
                command: key.to_string(),
                source_index: *source_index,
                version_condition: None,
            });
        }
        BazelRC {
            options: map,
            sources: sources.iter().map(|p| p.to_path_buf()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn announce_shortens_paths_by_anchor() {
        let root = Path::new("/work/repo");
        let home = Path::new("/home/dev");
        let rc = rc_with(
            &[
                &root.join(".bazelrc"),          // → ./.bazelrc
                &home.join(".bazelrc"),          // → ~/.bazelrc
                Path::new("/etc/bazel.bazelrc"), // → absolute
            ],
            &[
                ("common", 0, "--jobs=4"),
                ("common", 1, "--keep_going"),
                ("common", 2, "--curses=no"),
            ],
        );
        let out = rc.announce("build", false, 200, root, Some(home), |s| s.to_string());
        assert!(out.contains("./.bazelrc"), "{out}");
        assert!(out.contains("~/.bazelrc"), "{out}");
        assert!(out.contains("/etc/bazel.bazelrc"), "{out}");
        assert!(
            !out.contains("repo/.bazelrc"),
            "should not show last-2-components: {out}"
        );
    }

    #[test]
    fn announce_anchor_matches_path_segments_not_string_prefix() {
        // `/work/repo-foo` shares the string prefix `/work/repo` but is NOT a
        // path-segment descendant, so it must render absolute, not `./-foo/…`.
        let root = Path::new("/work/repo");
        let rc = rc_with(
            &[Path::new("/work/repo-foo/.bazelrc")],
            &[("common", 0, "--jobs=4")],
        );
        let out = rc.announce("build", false, 200, root, None, |s| s.to_string());
        assert!(out.contains("/work/repo-foo/.bazelrc"), "{out}");
        assert!(!out.contains("./"), "string-prefix must not anchor: {out}");
    }

    #[test]
    fn announce_anchor_prefers_root_when_root_under_home() {
        // root is itself under home; an rc under root must anchor to the more
        // specific `./`, not `~/repo/…` (root is checked first).
        let home = Path::new("/home/dev");
        let root = Path::new("/home/dev/repo");
        let rc = rc_with(&[&root.join(".bazelrc")], &[("common", 0, "--jobs=4")]);
        let out = rc.announce("build", false, 200, root, Some(home), |s| s.to_string());
        assert!(out.contains("./.bazelrc"), "{out}");
        assert!(
            !out.contains("~/repo"),
            "root anchor must win over home: {out}"
        );
    }

    #[test]
    fn announce_redacts_flag_values() {
        let root = Path::new("/work/repo");
        let rc = rc_with(
            &[&root.join(".bazelrc")],
            &[("common", 0, "--remote_header=Authorization: Bearer s3cr3t")],
        );
        // Redactor that drops everything after the first '=' for env/header-like flags.
        let redact = |flag: &str| -> String {
            match flag.split_once('=') {
                Some((name, _)) => format!("{name}=<REDACTED>"),
                None => flag.to_string(),
            }
        };
        let out = rc.announce("build", false, 200, root, None, redact);
        assert!(out.contains("--remote_header=<REDACTED>"), "{out}");
        assert!(!out.contains("s3cr3t"), "secret leaked: {out}");
    }

    #[test]
    fn announce_test_includes_inherited_build_flags() {
        // Bazel applies `build` (and `build:<config>`) flags to `test`, so the
        // announce for `test` must show them — not just the `test` sections.
        let root = Path::new("/work/repo");
        let rc = rc_with(
            &[&root.join(".bazelrc")],
            &[
                ("common", 0, "--curses=no"),
                ("build", 0, "--disk_cache=/cache"),
                ("test", 0, "--test_output=errors"),
                ("build:ci", 0, "--remote_cache=grpc://cache"),
                ("test:ci", 0, "--flaky_test_attempts=2"),
            ],
        );
        let out = rc.announce("test", false, 200, root, None, |s| s.to_string());
        assert!(
            out.contains("--disk_cache=/cache"),
            "inherited build flag missing: {out}"
        );
        assert!(
            out.contains("--test_output=errors"),
            "test flag missing: {out}"
        );
        assert!(
            out.contains("--remote_cache=grpc://cache"),
            "inherited build:<config> flag missing: {out}"
        );
        assert!(
            out.contains("--flaky_test_attempts=2"),
            "test:<config> flag missing: {out}"
        );
    }

    #[test]
    fn announce_config_sections_follow_expansion_order() {
        // For `test --config=ci` Bazel expands always:ci → common:ci →
        // build:ci → test:ci (last wins). The announce must list the same-named
        // config sections in that order, not alphabetically (which would put
        // build:ci before common:ci and misrepresent the override order).
        let root = Path::new("/work/repo");
        let rc = rc_with(
            &[&root.join(".bazelrc")],
            &[
                ("common:ci", 0, "--remote_cache=A"),
                ("build:ci", 0, "--remote_cache=B"),
                ("test:ci", 0, "--remote_cache=C"),
            ],
        );
        let out = rc.announce("test", false, 200, root, None, |s| s.to_string());
        let common_at = out.find("common:ci").expect("common:ci shown");
        let build_at = out.find("build:ci").expect("build:ci shown");
        let test_at = out.find("test:ci").expect("test:ci shown");
        assert!(
            common_at < build_at && build_at < test_at,
            "config sections must follow expansion order common→build→test: {out}"
        );
    }

    #[test]
    fn announce_build_omits_test_only_flags() {
        // The inheritance is one-directional: `build` must NOT show `test`
        // flags (test is a descendant of build, not an ancestor).
        let root = Path::new("/work/repo");
        let rc = rc_with(
            &[&root.join(".bazelrc")],
            &[
                ("build", 0, "--disk_cache=/cache"),
                ("test", 0, "--test_output=errors"),
            ],
        );
        let out = rc.announce("build", false, 200, root, None, |s| s.to_string());
        assert!(
            out.contains("--disk_cache=/cache"),
            "build flag missing: {out}"
        );
        assert!(
            !out.contains("--test_output=errors"),
            "build must not inherit test-only flags: {out}"
        );
    }

    // ── Config expansion ─────────────────────────────────────────────────────

    #[test]
    fn config_expansion_basic() {
        let dir = make_workspace();
        let root = dir.path();
        let rc_path = root.join(".bazelrc");
        fs::write(
            &rc_path,
            r#"
build:opt --copt=-O2
build:opt --compilation_mode=opt
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=opt"])).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert_eq!(values, vec!["--copt=-O2", "--compilation_mode=opt"]);
    }

    #[test]
    fn two_token_common_option_folds_into_single_eq_override() {
        // Regression: `common:ci --remote_download_outputs minimal` was emitted as
        // two `--default_override` flags, stranding `minimal` as its own
        // (unrecognized) option. It must fold into one canonical `=`-form override.
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            &root.join(".bazelrc"),
            "common:ci --remote_download_outputs minimal\n",
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=ci"])).unwrap();
        let (_startup, command_flags) = rc.resolve_for_command("build").unwrap();
        assert!(
            command_flags.contains(
                &"--default_override=0:common=--remote_download_outputs=minimal".to_string()
            ),
            "expected a single `=`-form override, got {command_flags:?}",
        );
    }

    #[test]
    fn config_expansion_cycle_detected() {
        let dir = make_workspace();
        let root = dir.path();
        let rc_path = root.join(".bazelrc");
        fs::write(
            &rc_path,
            r#"
build:a --config=b
build:b --config=a
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=a"])).unwrap();
        let err = rc.expand_configs("build", &[]).unwrap_err();
        assert!(matches!(err, BazelRcError::ConfigCycle { .. }));
    }

    #[test]
    fn config_undefined_errors() {
        let dir = make_workspace();
        let root = dir.path();
        let rc_path = root.join(".bazelrc");
        fs::write(&rc_path, "build --jobs=4\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=nonexistent"])).unwrap();
        let err = rc.expand_configs("build", &[]).unwrap_err();
        assert!(matches!(err, BazelRcError::UndefinedConfig { .. }));
    }

    #[test]
    fn build_only_config_is_undefined_for_query() {
        // `query` has no `build` ancestor, so a config defined only as
        // `build:<name>` is invisible to it and errors — even though the same
        // config resolves fine for `build`. (Regression context: the delivery
        // resolve query.)
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build:rel --copt=-O2\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=rel"])).unwrap();
        assert!(
            rc.expand_configs("build", &[]).is_ok(),
            "build sees build:rel"
        );
        let err = rc.expand_configs("query", &[]).unwrap_err();
        assert!(
            matches!(err, BazelRcError::UndefinedConfig { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn skip_config_if_missing_drops_undefined_config() {
        // Naming the config in `skip_config_if_missing` turns the otherwise
        // fatal undefined-config into a silent no-op — how the delivery query
        // tolerates a build-only `--config` it can't resolve.
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build:rel --copt=-O2\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=rel"])).unwrap();
        let expanded = rc.expand_configs("query", &["rel"]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert!(
            !values.contains(&"--copt=-O2"),
            "build:rel must not leak to query: {values:?}"
        );
    }

    // ── File discovery order and deduplication ───────────────────────────────

    #[test]
    fn discovery_deduplication() {
        let dir = make_workspace();
        let root = dir.path();
        let rc_path = root.join(".bazelrc");
        fs::write(&rc_path, "build --jobs=4\n").unwrap();

        // Passing the same path twice via --bazelrc should deduplicate.
        // Suppress system/home rcs so the test is hermetic regardless of the environment.
        let explicit = rc_path.display().to_string();
        let flags = vec![
            "--nosystem_rc".to_string(),
            "--nohome_rc".to_string(),
            format!("--bazelrc={explicit}"),
            format!("--bazelrc={explicit}"),
        ];
        let rc = BazelRC::new(root, &flags, &[]).unwrap();
        // workspace rc + 1 explicit (deduped from 2)
        assert_eq!(rc.sources().len(), 1);
    }

    #[test]
    fn nosystem_rc_skips_system() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4\n").unwrap();

        let rc = BazelRC::new(root, &["--nosystem_rc"], &[]).unwrap();
        // Should not error even though /etc/bazel.bazelrc likely doesn't exist (it's skipped)
        assert!(!rc.sources().is_empty() || rc.sources().is_empty()); // just no panic
    }

    #[test]
    fn options_for_precedence() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
always --always-flag
common --common-flag
build --build-flag
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let opts: Vec<&str> = rc
            .options_for("build")
            .iter()
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(opts, vec!["--always-flag", "--common-flag", "--build-flag"]);
    }

    #[test]
    fn test_inherits_build_flags() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
build --jobs=8
build --remote_cache=grpc://cache
test --test_output=errors
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let opts: Vec<&str> = rc
            .options_for("test")
            .iter()
            .map(|o| o.value.as_str())
            .collect();
        // test should include build flags before its own
        assert_eq!(
            opts,
            vec![
                "--jobs=8",
                "--remote_cache=grpc://cache",
                "--test_output=errors"
            ]
        );
    }

    #[test]
    fn test_config_inherits_build_config() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
build:opt --compilation_mode=opt
test:opt --test_output=errors
test --config=opt
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let expanded: Vec<String> = rc
            .expand_configs("test", &[])
            .unwrap()
            .into_iter()
            .map(|o| o.value)
            .collect();
        // --config=opt for test should expand build:opt AND test:opt
        assert!(
            expanded.contains(&"--compilation_mode=opt".to_owned()),
            "expected build:opt flags to be included; got: {expanded:?}"
        );
        assert!(
            expanded.contains(&"--test_output=errors".to_owned()),
            "expected test:opt flags to be included; got: {expanded:?}"
        );
    }

    #[test]
    fn cli_flags_appear_as_always_options() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4\n").unwrap();

        // ISOLATE so the runner's own ~/.bazelrc / /etc/bazel.bazelrc don't
        // leak into options_for and break the exact-equality assertion below.
        let rc = BazelRC::new(
            root,
            ISOLATE,
            &flags(&["--config=foo", "--verbose_failures"]),
        )
        .unwrap();

        // CLI flags appear at the END of options_for so they can override RC-file flags
        // under last-write-wins semantics (matches Bazel's CLI-overrides-RC behavior).
        let opts: Vec<&str> = rc
            .options_for("build")
            .iter()
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(opts, vec!["--jobs=4", "--config=foo", "--verbose_failures"]);

        // Source of CLI flags is the synthetic "<command line>" entry
        let always = rc.raw_options("always");
        assert_eq!(always.len(), 2);
        assert_eq!(rc.source_of(&always[0]), Path::new("<command line>"));
    }

    #[test]
    fn cli_config_flag_expands_via_rc() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build:opt --copt=-O2\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=opt"])).unwrap();
        // expand_configs picks up the CLI --config=opt from always options
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert_eq!(values, vec!["--copt=-O2"]);
    }

    // ── try-import-if-bazel-version ──────────────────────────────────────────

    #[test]
    fn versioned_import_tags_flags_with_condition() {
        let dir = make_workspace();
        let root = dir.path();

        let versioned = root.join("versioned.bazelrc");
        fs::write(&versioned, "build --sandbox_default_allow_network=false\n").unwrap();

        let main_rc = root.join(".bazelrc");
        fs::write(
            &main_rc,
            format!(
                r#"
build --jobs=4
try-import-if-bazel-version >=8.0.0 {}
"#,
                versioned.display()
            ),
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let opts = rc.options_for("build");

        let unconditional: Vec<&str> = opts
            .iter()
            .filter(|o| o.version_condition.is_none())
            .map(|o| o.value.as_str())
            .collect();
        let versioned_opts: Vec<(&str, &str)> = opts
            .iter()
            .filter_map(|o| {
                o.version_condition
                    .as_deref()
                    .map(|c| (o.value.as_str(), c))
            })
            .collect();

        assert_eq!(unconditional, vec!["--jobs=4"]);
        assert_eq!(
            versioned_opts,
            vec![("--sandbox_default_allow_network=false", ">=8.0.0")]
        );
    }

    #[test]
    fn versioned_import_missing_file_is_skipped() {
        let dir = make_workspace();
        let root = dir.path();

        let main_rc = root.join(".bazelrc");
        fs::write(
            &main_rc,
            r#"
build --jobs=4
try-import-if-bazel-version >=8.0.0 /nonexistent/path.bazelrc
"#,
        )
        .unwrap();

        // Should not error — missing file is silently skipped like try-import
        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let opts = rc.options_for("build");
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].value, "--jobs=4");
    }

    // ── expand_configs ordering and version_condition propagation ────────────

    #[test]
    fn expand_configs_preserves_order_across_versioned_and_unconditional() {
        let dir = make_workspace();
        let root = dir.path();

        let versioned = root.join("versioned.bazelrc");
        fs::write(&versioned, "build --sandbox_default_allow_network=false\n").unwrap();

        fs::write(
            root.join(".bazelrc"),
            format!(
                r#"
build --jobs=4
try-import-if-bazel-version >=8.0.0 {}
build --remote_cache=grpc://cache
"#,
                versioned.display()
            ),
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();

        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert_eq!(
            values,
            vec![
                "--jobs=4",
                "--sandbox_default_allow_network=false",
                "--remote_cache=grpc://cache"
            ]
        );

        // Middle entry is versioned
        assert_eq!(expanded[0].version_condition, None);
        assert_eq!(expanded[1].version_condition.as_deref(), Some(">=8.0.0"));
        assert_eq!(expanded[2].version_condition, None);
    }

    // ── File content parsing ──────────────────────────────────────────────────

    #[test]
    fn empty_file() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        assert_eq!(rc.options_for("build").len(), 0);
    }

    #[test]
    fn whitespace_only_file() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "   \n\t\n  \n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        assert_eq!(rc.options_for("build").len(), 0);
    }

    #[test]
    fn commented_line() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "# startup foo\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        assert_eq!(rc.options_for("startup").len(), 0);
    }

    #[test]
    fn command_with_no_args() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        assert_eq!(rc.options_for("build").len(), 0);
    }

    #[test]
    fn command_with_trailing_comment() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4 # a comment\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let opts = rc.options_for("build");
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].value, "--jobs=4");
    }

    #[test]
    fn multiple_args_on_one_line() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4 --verbose_failures\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let values: Vec<&str> = rc
            .options_for("build")
            .iter()
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(values, vec!["--jobs=4", "--verbose_failures"]);
    }

    #[test]
    fn multiple_lines_same_command_accumulates() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
build --jobs=4
build --verbose_failures
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let values: Vec<&str> = rc
            .options_for("build")
            .iter()
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(values, vec!["--jobs=4", "--verbose_failures"]);
    }

    #[test]
    fn multiple_different_commands() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
startup --max_idle_secs=60
build --jobs=4
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        assert_eq!(rc.options_for("startup").len(), 1);
        assert_eq!(rc.options_for("build").len(), 1);
        assert_eq!(rc.options_for("startup")[0].value, "--max_idle_secs=60");
        assert_eq!(rc.options_for("build")[0].value, "--jobs=4");
    }

    #[test]
    fn tab_separated_command_and_args() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build\t--jobs=4\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let opts = rc.options_for("build");
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].value, "--jobs=4");
    }

    #[test]
    fn indented_command_parsed() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "  build --jobs=4\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let opts = rc.options_for("build");
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].value, "--jobs=4");
    }

    #[test]
    fn line_continuation_in_bazelrc() {
        let dir = make_workspace();
        let root = dir.path();
        // Two separate build options joined by line continuation within a single directive
        fs::write(
            root.join(".bazelrc"),
            r#"build --jobs=4 \
  --verbose_failures
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let values: Vec<&str> = rc
            .options_for("build")
            .iter()
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(values, vec!["--jobs=4", "--verbose_failures"]);
    }

    // ── Import ordering ───────────────────────────────────────────────────────

    #[test]
    fn import_foo_then_add_bar() {
        // import before local flag → imported flags come before local flags
        let dir = make_workspace();
        let root = dir.path();

        let foo = root.join("foo.bazelrc");
        fs::write(&foo, "build --from-foo\n").unwrap();

        fs::write(
            root.join(".bazelrc"),
            format!(
                r#"import {}
build --local-bar
"#,
                foo.display()
            ),
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let values: Vec<&str> = rc
            .options_for("build")
            .iter()
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(values, vec!["--from-foo", "--local-bar"]);
    }

    #[test]
    fn add_bar_then_import_foo() {
        // local flag before import → local flag comes first
        let dir = make_workspace();
        let root = dir.path();

        let foo = root.join("foo.bazelrc");
        fs::write(&foo, "build --from-foo\n").unwrap();

        fs::write(
            root.join(".bazelrc"),
            format!(
                r#"build --local-bar
import {}
"#,
                foo.display()
            ),
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let values: Vec<&str> = rc
            .options_for("build")
            .iter()
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(values, vec!["--local-bar", "--from-foo"]);
    }

    // ── File discovery ────────────────────────────────────────────────────────

    #[test]
    fn noworkspace_rc_skips_workspace() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --workspace-flag\n").unwrap();

        let rc = BazelRC::new(
            root,
            &["--nosystem_rc", "--nohome_rc", "--noworkspace_rc"],
            &[],
        )
        .unwrap();
        // Workspace .bazelrc should not be loaded
        assert!(rc.raw_options("build").is_empty());
    }

    #[test]
    fn nohome_rc_skips_home() {
        let dir = make_workspace();
        let root = dir.path();
        // Without a workspace .bazelrc, just verify --nohome_rc is accepted without error
        let rc = BazelRC::new(root, &["--nohome_rc"], &[]);
        assert!(rc.is_ok());
    }

    #[test]
    fn ignore_all_rc_files_skips_all() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --workspace-flag\n").unwrap();

        let rc = BazelRC::new(root, &["--ignore_all_rc_files"], &[]).unwrap();
        assert!(rc.sources().is_empty());
        assert!(rc.options_for("build").is_empty());
    }

    #[test]
    fn explicit_bazelrc_loads_file() {
        let dir = make_workspace();
        let root = dir.path();
        let explicit = root.join("custom.bazelrc");
        fs::write(&explicit, "build --custom-flag\n").unwrap();

        let flag = format!("--bazelrc={}", explicit.display());
        // Suppress all auto-discovered rcs so only the explicit file is loaded.
        let rc = BazelRC::new(
            root,
            &[
                "--nosystem_rc",
                "--noworkspace_rc",
                "--nohome_rc",
                flag.as_str(),
            ],
            &[],
        )
        .unwrap();
        assert_eq!(rc.sources().len(), 1);
        assert_eq!(rc.options_for("build")[0].value, "--custom-flag");
    }

    #[test]
    fn explicit_bazelrc_missing_errors() {
        let dir = make_workspace();
        let root = dir.path();

        let err = BazelRC::new(root, &["--bazelrc=/nonexistent/missing.bazelrc"], &[]).unwrap_err();
        assert!(matches!(err, BazelRcError::BazelrcNotFound { .. }));
    }

    // ── Error cases ───────────────────────────────────────────────────────────

    #[test]
    fn import_too_many_args_error() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "import foo bar\n").unwrap();

        let err = BazelRC::new(root, ISOLATE, &[]).unwrap_err();
        assert!(matches!(err, BazelRcError::InvalidImportArgs { .. }));
    }

    #[test]
    fn try_import_too_many_args_error() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "try-import foo bar\n").unwrap();

        let err = BazelRC::new(root, ISOLATE, &[]).unwrap_err();
        assert!(matches!(err, BazelRcError::InvalidImportArgs { .. }));
    }

    // ── Config expansion ──────────────────────────────────────────────────────

    #[test]
    fn common_config_section_used_as_fallback() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "common:myconfig --common-flag\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=myconfig"])).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert!(values.contains(&"--common-flag"), "got: {values:?}");
    }

    #[test]
    fn multi_level_config_expansion() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
build:a --config=b
build:b --deep-flag
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=a"])).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert_eq!(values, vec!["--deep-flag"]);
    }

    // ── Config vs non-config ordering (Bug #1) ───────────────────────────────

    #[test]
    fn config_expands_in_place() {
        // Bazel expands --config=foo in-place at the position it appears.
        // Flags that come *after* --config=opt in the rc file appear after the
        // expansion, so they win under last-write-wins — matching Bazel's spec:
        // https://bazel.build/versions/9.0.0/run/bazelrc#option-defaults
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
build:opt --config-flag
build --non-config-before
build --config=opt
build --non-config-after
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();

        // Expected in-place order: before, <config expansion>, after
        assert_eq!(
            values,
            vec!["--non-config-before", "--config-flag", "--non-config-after"],
            "got: {values:?}"
        );
    }

    #[test]
    fn config_flags_preserve_file_ordering() {
        // When the same config section is defined in two files, the flag from the
        // later file must appear last in the expansion so it wins (last-write-wins).
        let dir = make_workspace();
        let root = dir.path();

        let second = root.join("second.bazelrc");
        fs::write(&second, "build:opt --flag=from-second\n").unwrap();

        fs::write(
            root.join(".bazelrc"),
            format!(
                r#"build:opt --flag=from-first
import {}
"#,
                second.display()
            ),
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=opt"])).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();

        assert_eq!(values, vec!["--flag=from-first", "--flag=from-second"]);
    }

    #[test]
    fn multiple_configs_each_come_after_non_config_flags() {
        // When multiple --config= flags are present, all their expansions must
        // appear after all non-config flags, and the configs' relative order is
        // the order the --config= flags appear in the rc file.
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
build:foo --foo-flag
build:bar --bar-flag
build --non-config
build --config=foo
build --config=bar
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();

        assert_eq!(values, vec!["--non-config", "--foo-flag", "--bar-flag"]);
    }

    #[test]
    fn enable_platform_specific_config() {
        let dir = make_workspace();
        let root = dir.path();
        let os = std::env::consts::OS;
        fs::write(
            root.join(".bazelrc"),
            format!("build:{os} --os-specific-flag\n"),
        )
        .unwrap();

        let rc = BazelRC::new(
            root,
            &[] as &[&str],
            &flags(&["--enable_platform_specific_config"]),
        )
        .unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert!(
            values.contains(&"--os-specific-flag"),
            "expected --os-specific-flag in {values:?}"
        );
    }

    #[test]
    fn config_in_common_section_expands() {
        // common --config=foo with build:foo defined → build expansion includes foo's flags
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
common --config=foo
build:foo --foo-flag
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert!(values.contains(&"--foo-flag"), "got: {values:?}");
    }

    #[test]
    fn config_in_always_section_expands() {
        // always --config=foo with build:foo defined → build expansion includes foo's flags
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
always --config=foo
build:foo --foo-flag
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert!(values.contains(&"--foo-flag"), "got: {values:?}");
    }

    #[test]
    fn config_defined_in_always_colon_key() {
        // always:myconfig --flag used when build:myconfig and common:myconfig are absent
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            "always:myconfig --always-only-flag\n",
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=myconfig"])).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert!(values.contains(&"--always-only-flag"), "got: {values:?}");
    }

    #[test]
    fn enable_platform_specific_config_no_section_is_silent() {
        // --enable_platform_specific_config with no build:<os> section must not error
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4\n").unwrap();

        let rc = BazelRC::new(
            root,
            &[] as &[&str],
            &flags(&["--enable_platform_specific_config"]),
        )
        .unwrap();
        // Should succeed and return the regular flag, not UndefinedConfig
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert!(values.contains(&"--jobs=4"));
    }

    #[test]
    fn ignore_all_rc_files_suppresses_explicit_bazelrc() {
        // --ignore_all_rc_files must suppress --bazelrc= files too (Bazel spec)
        let dir = make_workspace();
        let root = dir.path();
        let explicit = root.join("explicit.bazelrc");
        fs::write(&explicit, "build --explicit-flag\n").unwrap();

        let flag = format!("--bazelrc={}", explicit.display());
        let rc = BazelRC::new(root, &["--ignore_all_rc_files", flag.as_str()], &[]).unwrap();
        assert!(rc.sources().is_empty());
        assert!(rc.options_for("build").is_empty());
    }

    #[test]
    fn unconditional_config_resolving_to_versioned_section_inherits_condition() {
        let dir = make_workspace();
        let root = dir.path();

        let versioned = root.join("versioned.bazelrc");
        // This file's flags will be tagged >=8.0.0
        fs::write(
            &versioned,
            "build:myconfig --sandbox_default_allow_network=false\n",
        )
        .unwrap();

        fs::write(
            root.join(".bazelrc"),
            format!(
                "try-import-if-bazel-version >=8.0.0 {}\n",
                versioned.display()
            ),
        )
        .unwrap();

        // The unconditional --config=myconfig triggers expansion of the versioned section
        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=myconfig"])).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].value, "--sandbox_default_allow_network=false");
        // Inherited from the versioned config section
        assert_eq!(expanded[0].version_condition.as_deref(), Some(">=8.0.0"));
    }

    // ── CLI --config= overrides unconditional common flags (Bug #3) ──────────

    #[test]
    fn cli_config_overrides_common_flag() {
        // When --config=ci is passed as a CLI flag (via the flags parameter), the ci-specific
        // flags must appear AFTER the unconditional common flags so they win under
        // last-write-wins — matching Bazel's CLI-overrides-RC semantics.
        //
        // Regression test for the monopi remote_timeout bug:
        //   common --remote_timeout=600          ← RC default
        //   common:ci --remote_timeout=3600      ← CI override
        // With --config=ci, 3600 must win.
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
common --remote_timeout=600
common:ci --remote_timeout=3600
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=ci"])).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();

        // 600 (RC default) first, 3600 (CI override) last — so 3600 wins
        assert_eq!(
            values,
            vec!["--remote_timeout=600", "--remote_timeout=3600"],
        );
    }

    #[test]
    fn cli_config_modify_execution_info_order() {
        // Regression test for the monopi Tar caching bug:
        //   common --modify_execution_info=Tar=+no-remote-cache   ← local default
        //   common:ci --modify_execution_info=Tar=-no-remote-cache ← CI override (allow hits)
        // With --config=ci, the `-` (remove) must come AFTER the `+` (add) so
        // Tar actions get remote-cache hits on CI.
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
common --modify_execution_info=Tar=+no-remote-cache
common:ci --modify_execution_info=Tar=-no-remote-cache
"#,
        )
        .unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=ci"])).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();

        // `+` (local default) first, `-` (CI override, allows cache hits) last
        assert_eq!(
            values,
            vec![
                "--modify_execution_info=Tar=+no-remote-cache",
                "--modify_execution_info=Tar=-no-remote-cache",
            ],
        );
    }
}
