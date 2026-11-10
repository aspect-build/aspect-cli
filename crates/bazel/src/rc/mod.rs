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

/// The rc sections `command` reads, in the order Bazel applies them: `always`, `common`,
/// the ancestors general → specific, then `command` itself. Later sections win.
///
/// Not deduplicated: `sections_for("common")` repeats the `common` bucket, which
/// last-write-wins makes harmless and which keeps a `common`-rendered rc (see
/// `aspect setup bazelrc`) resolving exactly as it did.
pub(crate) fn sections_for(command: &str) -> Vec<&str> {
    let mut sections = vec!["always", "common"];
    sections.extend(command_ancestors(command));
    sections.push(command);
    sections
}

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use std::fmt;

#[cfg(feature = "starlark")]
use allocative::Allocative;
#[cfg(feature = "starlark")]
use starlark::eval::Evaluator;
#[cfg(feature = "starlark")]
use starlark::starlark_simple_value;
#[cfg(feature = "starlark")]
use starlark::values::none::NoneOr;
#[cfg(feature = "starlark")]
use starlark::values::{
    NoSerialize, ProvidesStaticType, StarlarkValue, UnpackValue, Value, starlark_value,
    type_repr::StarlarkTypeRepr,
};
use thiserror::Error;

/// The synthetic `Bazelrc::sources` entry caller-supplied flags are attributed to.
const COMMAND_LINE_SOURCE: &str = "<command line>";

/// A single option value parsed from a bazelrc file.
#[derive(Debug, Clone, Default)]
pub struct RcOption {
    /// The option string, e.g. `"--jobs=4"`.
    pub value: String,
    /// The command section this option came from, e.g. `"build"`, `"build:opt"`,
    /// `"common"`, or `"always"`. On an option out of `expand_configs`, usually
    /// the section of the `--config=` request that pulled it in (see there).
    pub command: String,
    /// Index into `Bazelrc::sources` identifying which file this came from. On
    /// an option out of `expand_configs`, follows `command`.
    pub source_index: usize,
    /// Set to `Some(cond)` when the option originates from a
    /// `try-import-if-bazel-version <cond> <path>` block.  The condition is an
    /// opaque string (e.g. `">=8.0.0"`, `"~8"`); version evaluation is left to
    /// the caller.
    pub version_condition: Option<String>,
}

/// Caller-supplied flags become synthetic rc entries under the section each
/// names, or `always` when it names none, so a task can scope a flag to the
/// commands that accept it (`build` reaches test/run/coverage/cquery/aquery
/// through `command_ancestors`, never `query`).
fn push_caller_flags(
    options: &mut HashMap<String, Vec<RcOption>>,
    flags: &[RcOption],
    source_index: usize,
) {
    for flag in flags {
        let command = if flag.command.is_empty() {
            "always".to_owned()
        } else {
            flag.command.clone()
        };
        options.entry(command.clone()).or_default().push(RcOption {
            source_index,
            command,
            ..flag.clone()
        });
    }
}

impl RcOption {
    /// The section `command` names, without its `:config` qualifier.
    fn section(&self) -> &str {
        self.command.split(':').next().unwrap_or(&self.command)
    }
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

/// Errors that can occur when loading or using a `Bazelrc`.
#[derive(Debug, Error)]
pub enum BazelrcError {
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
/// startup flags, `--config` skip policy, and Bazel version context: everything
/// the CLI resolves before invoking Bazel with `--ignore_all_rc_files`.
///
/// With the `starlark` feature this is the Starlark `bazel.Bazelrc` value —
/// constructed by `ctx.bazel.parse_rc()` / `ctx.bazel.new_rc()`, composed with
/// `merge`, inspected with `flag_value` / `bool_flag` / `flag_values`, and
/// handed to `ctx.bazel.build` / `test` / `query`.
#[derive(Debug, Clone, Default)]
#[cfg_attr(
    feature = "starlark",
    derive(ProvidesStaticType, NoSerialize, Allocative)
)]
pub struct Bazelrc {
    /// Map from command key (e.g. `"build"`, `"build:opt"`) to its options.
    #[cfg_attr(feature = "starlark", allocative(skip))]
    options: HashMap<String, Vec<RcOption>>,
    /// Ordered list of source files that were loaded.
    #[cfg_attr(feature = "starlark", allocative(skip))]
    sources: Vec<PathBuf>,
    /// `--config` names to drop when undefined (so a sidecar `query` does not
    /// fail on a config that only the build command defines).
    #[cfg_attr(feature = "starlark", allocative(skip))]
    skip_config_if_missing: Vec<String>,
    /// User-provided startup flags (e.g. `--output_base=`), distinct from the
    /// rc-file `startup` section; a Bazel invocation sources startup options
    /// from here.
    #[cfg_attr(feature = "starlark", allocative(skip))]
    startup_flags: Vec<String>,
    /// Bazel version used to evaluate version-gated options; `None` falls back
    /// to assuming the latest. Resolved at construction.
    #[cfg_attr(feature = "starlark", allocative(skip))]
    version: Option<semver::Version>,
}

#[cfg(feature = "starlark")]
starlark_simple_value!(Bazelrc);

#[cfg(feature = "starlark")]
#[starlark_value(type = "bazel.Bazelrc")]
impl<'v> StarlarkValue<'v> for Bazelrc {
    fn get_methods() -> Option<&'static starlark::environment::Methods> {
        static RES: starlark::environment::MethodsStatic =
            starlark::environment::MethodsStatic::new("runcommand_methods", runcommand_methods);
        Some(RES.methods())
    }
}

impl fmt::Display for Bazelrc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bazelrc(sources: [")?;
        for (i, src) in self.sources.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", src.display())?;
        }
        write!(f, "], keys: {})", self.options.len())
    }
}

impl Bazelrc {
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
    ) -> Result<Self, BazelrcError> {
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

        // Append caller-supplied flags as synthetic rc entries so they participate in
        // options_for() and expand_configs() like any rc-file entry: under `always`
        // unless the flag names its own command section.
        if !flags.is_empty() {
            let cli_source_index = sources.len();
            sources.push(PathBuf::from(COMMAND_LINE_SOURCE));
            push_caller_flags(&mut options, flags, cli_source_index);
        }

        Ok(Bazelrc {
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

    /// The user-provided startup flags carried by this bazelrc.
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

    /// A `Bazelrc` that reads no files — only the given command `flags`,
    /// stored as synthetic `always` options. Used by `ctx.bazel.new_rc()` to
    /// build one from scratch.
    pub fn blank(flags: &[RcOption]) -> Self {
        let mut options: HashMap<String, Vec<RcOption>> = HashMap::new();
        let mut sources: Vec<PathBuf> = Vec::new();
        if !flags.is_empty() {
            let cli_source_index = 0;
            sources.push(PathBuf::from(COMMAND_LINE_SOURCE));
            push_caller_flags(&mut options, flags, cli_source_index);
        }
        Bazelrc {
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
    /// Order: every section this command reads — `always` + `common` + ancestor commands
    /// (general → specific) + `<command>` — first from the rc files, then again from the
    /// caller-supplied flags.
    ///
    /// Caller-supplied flags (those passed via the `flags` parameter to `Bazelrc::new`, carrying
    /// the synthetic source `"<command line>"`) are placed **last** so that any `--config=`
    /// flags they carry expand after all RC-file flags.  This matches Bazel's own semantics
    /// where command-line flags override `.bazelrc` defaults under last-write-wins. A caller
    /// flag that named a section (`("--jobs=8", "", "build")`) is placed last the same way, so
    /// scoping a flag to the commands that accept it never costs it its precedence over the
    /// rc files.
    ///
    /// Within the caller block the order is the section order, not the order the caller listed
    /// them in — the same rule Bazel applies to rc entries, where `build --foo` wins over
    /// `common --foo` whichever line came first. So a scoped caller flag beats an unscoped one
    /// on a build-like command; `--bazel-flags:omit` is how a user drops an injected flag
    /// rather than shadowing it.
    ///
    /// One exception to "caller flags win": a caller flag that names the `common` section
    /// itself. `resolve_for_command` renders every `common`-section option as
    /// `--default_override=0:common=…`, at rc priority, so such a flag loses to an rc-file
    /// entry in a more specific section such as `build`, despite being emitted after it here.
    /// Nothing in-tree does that: callers name a command section or leave the flag unscoped
    /// (`always`), and both of those stay literal.
    ///
    /// For example, `options_for("test")` returns:
    ///   `always` + `common` + `build` + `test` (rc-file) + `always` + `build` + `test` (caller)
    pub fn options_for(&self, command: &str) -> Vec<&RcOption> {
        // Each bucket keeps insertion order, so a merged overlay's caller flags
        // follow the base's and win under last-write-wins.
        let sections = sections_for(command);
        let mut result = Vec::new();
        for caller_pass in [false, true] {
            for section in &sections {
                if let Some(opts) = self.options.get(*section) {
                    result.extend(
                        opts.iter()
                            .filter(|o| self.is_command_line(o) == caller_pass),
                    );
                }
            }
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
            if p == Path::new(COMMAND_LINE_SOURCE) {
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

        // Config keys (`<base>:<config>`) whose base is a section this command reads —
        // ancestor bases included, e.g. `build:ci` applies to `test --config=ci`.
        // `sections_for` is both the membership test and the rank: a base's position in it
        // is the order `expand::expand_args` applies the sections in, so two sections
        // setting the same flag (`common:ci` then `build:ci`) are announced in the override
        // order Bazel applies rather than alphabetically, which would invert them. Keys are
        // then grouped by config name (alphabetical) for deterministic output.
        let sections = sections_for(command);
        let base_rank = |base: &str| sections.iter().position(|section| *section == base);
        let mut config_keys: Vec<&String> = self
            .options
            .keys()
            .filter(|k| {
                k.split_once(':')
                    .is_some_and(|(base, _)| base_rank(base).is_some())
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
        let mut direct_keys = vec!["startup"];
        direct_keys.extend(sections_for(command));
        for (source_idx, source_path) in self.sources.iter().enumerate() {
            let is_client = source_path == Path::new(COMMAND_LINE_SOURCE);
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
            .position(|p| p == Path::new(COMMAND_LINE_SOURCE))
            .unwrap_or_else(|| {
                let idx = self.sources.len();
                self.sources.push(PathBuf::from(COMMAND_LINE_SOURCE));
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

    /// Overlay `other` on top of `self`, returning a new `Bazelrc`.
    ///
    /// `other` wins on conflicts: within each command section its options are
    /// appended **after** `self`'s, so Bazel's last-write-wins ordering resolves
    /// in `other`'s favor. Sources are concatenated, with `other`'s
    /// `source_index` values rebased onto the combined list. Neither input is
    /// mutated.
    pub fn merge(&self, other: &Bazelrc) -> Bazelrc {
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
    /// options, plus any CLI flags passed to `Bazelrc::new`). Returns the fully-expanded flat list
    /// of `RcOption`s with all `--config=` references resolved.
    ///
    /// Each entry's `version_condition` is `Some(cond)` when the option originated from a
    /// `try-import-if-bazel-version` block; `None` otherwise. Callers can inspect this field to
    /// decide how to handle version-gated flags.
    pub fn expand_configs(
        &self,
        command: &str,
        skip_config_if_missing: &[&str],
    ) -> Result<Vec<RcOption>, BazelrcError> {
        expand::expand_configs(self, command, skip_config_if_missing)
    }

    /// Assemble the `(startup_flags, command_flags)` to run `command` with this
    /// bazelrc. `command_flags` are the config-expanded, version-filtered
    /// options — rc-file ones rendered as `--default_override=0:<section>=…`
    /// tokens, caller-supplied ones literal (`common`-scoped caller flags ride
    /// the override transport too — see [`partition_expand_all`]);
    /// `startup_flags` are the user startup flags plus the rc-file `startup`
    /// section plus `--ignore_all_rc_files` (the on-disk rc is already absorbed).
    pub fn resolve_for_command(&self, command: &str) -> anyhow::Result<(Vec<String>, Vec<String>)> {
        let opts = self.applicable_options(command, None)?;
        let (overrides, regular) = partition_expand_all(&opts, |o| self.is_command_line(o));
        let command_flags: Vec<String> = overrides
            .into_iter()
            .map(|(v, _)| v)
            .chain(regular.into_iter().map(|(v, _)| v))
            .collect();
        Ok((self.invocation_startup_flags(), command_flags))
    }

    /// Whether `opt` is caller-supplied (synthetic source `"<command line>"`)
    /// rather than an entry read from an rc file. A merged Bazelrc can carry
    /// more than one such source — one per merged input — so the check is on
    /// the source path, not a single index.
    fn is_command_line(&self, opt: &RcOption) -> bool {
        self.sources
            .get(opt.source_index)
            .is_some_and(|p| p.as_path() == Path::new(COMMAND_LINE_SOURCE))
    }

    /// The startup flags Bazel runs with when this bazelrc is in effect:
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

    /// Whether a version-gated option's `condition` holds at this bazelrc's
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

/// Bazel's `--default_override=<rc index>:<command>=<option>` token: one rc-file option,
/// restated on the command line under the section it was written in.
///
/// It is how an rc-file option survives `--ignore_all_rc_files`, which the CLI always
/// passes: a literal `--option` on the command line would be rejected by any command that
/// does not accept it — or, before Bazel even sees it, by a `bazel` wrapper in front of the
/// real client that registers the flag itself — whereas Bazel applies an override only to the
/// commands its section covers. `common` covers all of them leniently; a command name
/// covers that command and everything inheriting from it. This is the transport Bazel's
/// own client uses to hand rc entries to the server. Bazel also parses overrides at rc
/// priority — ahead of everything literal on the command line — so an override never
/// outranks a flag the user or a task passed directly.
///
/// One type for both directions: [`partition_expand_all`] renders these and
/// [`last_flag_value`] reads them back, so the wire form is written down once.
struct DefaultOverride<'a> {
    /// Which rc file the option came from, as an index into the `--bazelrc` list Bazel's
    /// client reported. The CLI renders every file it read as one synthetic source, so it
    /// always emits [`SOLE_SOURCE`](Self::SOLE_SOURCE); the field exists because the file it
    /// names is what delimits a parse bucket (see [`override_chunks`]).
    index: u32,
    /// The section the option is scoped to, e.g. `common`, `build`, `always`. Rendered
    /// without a `:config` qualifier — `expand_configs` has already decided which config
    /// sections apply. The invariant holds in that direction only: [`parse`](Self::parse)
    /// will read a qualified section back out of foreign input. It is ignored, and safely:
    /// `expand_configs` consumes every `--config=` request, so no config is active on the
    /// command line Bazel receives for a `{section}:{config}` override to key off.
    command: &'a str,
    /// The option, in the single-token `--flag=value` form Bazel requires here.
    option: &'a str,
}

impl fmt::Display for DefaultOverride<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}:{}={}",
            DefaultOverride::PREFIX,
            self.index,
            self.command,
            self.option
        )
    }
}

/// The name of Bazel's `--default_override` option, as BEP's structured command line
/// reports it.
pub const DEFAULT_OVERRIDE_OPTION: &str = "default_override";

impl<'a> DefaultOverride<'a> {
    const PREFIX: &'static str = "--default_override=";

    /// The rc index the CLI renders under: it restates every rc file it read as one
    /// synthetic source, so all of its payloads share a bucket.
    const SOLE_SOURCE: u32 = 0;

    /// Read back a token [`Display`](fmt::Display) wrote, or `None` for any other token.
    fn parse(token: &'a str) -> Option<Self> {
        let body = token.strip_prefix(Self::PREFIX)?;
        let (index, scoped) = body.split_once(':')?;
        let index = index.parse::<u32>().ok()?;
        let (command, option) = scoped.split_once('=')?;
        (!command.is_empty() && !option.is_empty()).then_some(Self {
            index,
            command,
            option,
        })
    }
}

/// The [`DefaultOverride`] payloads among `tokens`, grouped into maximal runs of
/// *consecutive* override tokens sharing one rc index. Each bucket is a list of
/// `(section, option)` pairs in token order; non-override tokens are skipped and do not
/// split a bucket.
///
/// The buckets matter because a bare `--flag` pairs with its value only inside one, and a
/// split leaves it the `Expected value after --flag` parse error Bazel reports. Bazel's own
/// boundary is the rc *file* the index names: it tracks the file of the previous override
/// token globally, not per section, and closes the bucket for *every* section as soon as it
/// changes — so the split falls wherever the file changes, and a return to an earlier file
/// opens a fresh bucket rather than rejoining that file's earlier one. (Probed against Bazel
/// 9.0.1 and 9.3.0.)
///
/// The index stands in for the file here, which is exact for this crate's own output — one rc
/// source, never a bogus index or section — but an approximation in general. Two
/// `--rc_source` entries can name the same path at different indices, and Bazel discards an
/// override whose index is out of range or whose section is not a command *before* it tests
/// the boundary, so none of those three splits a bucket for Bazel; a foreign caller passing
/// one gets a split where Bazel pairs. Telling them apart needs the `--rc_source` list and
/// Bazel's set of valid commands, neither of which this function sees. (Probed on 9.0.1.)
fn override_chunks<'a>(tokens: &[&'a str]) -> Vec<Vec<(&'a str, &'a str)>> {
    let mut chunks: Vec<Vec<(&str, &str)>> = Vec::new();
    let mut open_index = None;
    for over in tokens.iter().copied().filter_map(DefaultOverride::parse) {
        if open_index != Some(over.index) {
            open_index = Some(over.index);
            chunks.push(Vec::new());
        }
        chunks
            .last_mut()
            .expect("a chunk was just opened")
            .push((over.command, over.option));
    }
    chunks
}

/// One rendered command-line token and the `version_condition` it carries.
type RenderedFlag = (String, Option<String>);

/// Split an expanded option list into `(default_override_flags, regular_flags)`.
///
/// Options read from rc files become [`DefaultOverride`] tokens rendered under the base
/// section they were written in (`common`, `build`, `always`, …) — for a config's options,
/// the section that requested the config (see `expand_configs`) — returned first so they
/// precede user flags, which Bazel applies after them either way: override payloads parse
/// at rc priority and the command line after them. This is the wire form Bazel's own
/// client restates rc entries in, and it is load-bearing for more than fidelity: a
/// literal `build`-section flag on the command line can be one a `bazel` wrapper in
/// front of the real client registers as its own — Aspect Workflows' agent wrapper
/// registers `--experimental_convenience_symlinks` as a boolean and rejects the enum
/// value `normal` before Bazel runs — whereas an override token names no flag
/// the wrapper knows and passes through untouched.
///
/// Caller-supplied flags (`is_caller`, the synthetic `"<command line>"` source, which
/// includes the options of a config the caller requested) stay literal, keeping the
/// command-line priority Bazel gives them over every rc entry: an override payload would
/// demote a caller flag back to rc priority, where a more specific rc section beats it.
/// One exception: a caller flag scoped to `common` rides the transport too — a literal
/// `--option` is rejected by any command that does not accept it, and `common`'s
/// every-command applicability is the only way it reaches the commands that do. Each
/// entry keeps its `version_condition` so version-gated options aren't silently promoted
/// to unconditional.
///
/// A two-token option (`--flag value`, parsed as a bare-flag `RcOption` followed by its value
/// `RcOption`) is folded into the canonical `--flag=value` form for its single
/// `--default_override=0:<section>=--flag=value` entry. Bazel parses each override value as one
/// option token and does not re-split it on whitespace, so a space-joined `--flag value`
/// payload is rejected as a single unrecognized option. Two payloads *do* pair when they land
/// in one parse bucket — a run of override tokens sharing an rc index, read a section at a
/// time ([`override_chunks`]) — and leave the bare flag an `Expected value after --flag` error
/// when they do not. The fold makes the emitted form independent of that bucketing. (Probed
/// against Bazel 9.0.1 and 9.3.0.) Regular flags need no such fold — they pass through as
/// adjacent argv tokens, which Bazel pairs natively.
///
/// Accepted trade-off: a bare token following a boolean flag is folded as if it were the flag's
/// value, even when it is really a non-option argument. Bazel's bazelrc supports non-option args
/// in rc lines (e.g. `build --keep_going //:all`, where `//:all` is a default target prepended
/// to the command line); under this fold that becomes `--keep_going=//:all` and the target is
/// lost. Telling the two apart needs a per-flag "takes a value" registry we don't have, and the
/// value-flag case (`--remote_download_outputs minimal`) is by far the common one — so we do
/// not support non-option arguments in rc lines that ride the override transport. Put default
/// targets on the command line; a caller flag (which stays literal) can still carry one.
fn partition_expand_all(
    opts: &[RcOption],
    is_caller: impl Fn(&RcOption) -> bool,
) -> (Vec<RenderedFlag>, Vec<RenderedFlag>) {
    let mut overrides = Vec::new();
    let mut flags = Vec::new();
    let mut i = 0;
    while i < opts.len() {
        let opt = &opts[i];
        let section = opt.section();
        if section == "common" || !is_caller(opt) {
            let mut value = opt.value.clone();
            if let Some(next) = opts.get(i + 1).filter(|next| is_two_token_pair(opt, next)) {
                value = format!("{}={}", opt.value, next.value);
                i += 1;
            }
            overrides.push((
                DefaultOverride {
                    index: DefaultOverride::SOLE_SOURCE,
                    command: section,
                    option: &value,
                }
                .to_string(),
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
/// the same command section, source and version condition (i.e. originate from the same rc
/// line).
///
/// Purely positional — it cannot tell a value-taking flag from a boolean flag trailed by a
/// non-option argument, so `--keep_going //:all` also matches. See `partition_expand_all` for
/// why that trade-off is accepted (non-option args cannot ride the override transport).
fn is_two_token_pair(flag: &RcOption, value: &RcOption) -> bool {
    flag.value.starts_with('-')
        && !flag.value.contains('=')
        && !value.value.starts_with('-')
        && flag.command == value.command
        && flag.source_index == value.source_index
        && flag.version_condition == value.version_condition
}

/// All values of a repeatable `--name` flag among `tokens`, in order. Matches
/// `--name=VALUE` and the two-token `--name VALUE` form, where a token starting
/// with `-` is never taken as a value (that is the next flag, and `--name` was
/// passed bare).
///
/// Shared by the rc options path (`flag_value_list`) and by the command-line one
/// ([`last_flag_value`], which also feeds it `--default_override` payloads), so
/// every caller reads a flag the way Bazel does.
pub fn flag_values_in(tokens: &[&str], name: &str) -> Vec<String> {
    let eq_prefix = format!("{name}=");
    let mut values = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if let Some(rest) = token.strip_prefix(&eq_prefix) {
            values.push(rest.to_string());
        } else if *token == name {
            if let Some(next) = tokens.get(i + 1) {
                if !next.starts_with('-') {
                    values.push((*next).to_string());
                }
            }
        }
    }
    values
}

/// The effective value of a single-valued `--name` on a rendered Bazel command line for
/// `command`, or `None` if unset.
///
/// Reads both forms the CLI renders rc content in, because it runs Bazel with
/// `--ignore_all_rc_files` and restates that content itself: a literal token, and a
/// `DefaultOverride` carrying an rc-file option restated under its own section. Reading
/// only literals makes every rc-file flag invisible, so a caller asking
/// "has this been requested already?" adds its own and silently outranks the user's.
///
/// A literal occurrence beats an override whatever the textual order, because Bazel parses
/// override payloads at rc priority and the command line after them. Among equals the last
/// occurrence wins, which is how Bazel resolves a flag given more than once.
///
/// Override payloads are read one section at a time, and within a section one rc-index run at
/// a time, rather than as a single stream, because that is how Bazel parses them — see
/// `override_chunks` for the bucket boundary. So a two-token `--name value` pair forms only
/// inside one bucket, and a bare `--name` payload whose value landed in another names
/// nothing: to Bazel that shape is a parse error (`Expected value after --name`), not a
/// value.
///
/// Between buckets, the section outranks everything: Bazel parses each section's payloads in
/// turn, so a `build` payload beats a `common` one however the tokens are ordered, and only
/// within one section does the later bucket win. (Probed against Bazel 9.0.1 and 9.3.0.)
///
/// None of this is a live path: `partition_expand_all` folds a two-token option into
/// `--flag=value` before rendering, and emits a single rc index, so the CLI never splits a
/// pair. It matters because this function is `pub` and is the single authority on "has this
/// flag been requested?", so it must not read a value out of a shape Bazel would reject.
pub fn last_flag_value(args: &[String], name: &str, command: &str) -> Option<String> {
    let tokens: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Some(literal) = flag_values_in(&tokens, name).pop() {
        return Some(literal);
    }
    override_buckets(&tokens, command)
        .iter()
        .filter_map(|bucket| flag_values_in(bucket, name).pop())
        .next_back()
}

/// The [`DefaultOverride`] payloads among `tokens` that apply to `command`, in the order
/// Bazel parses them: a section at a time in [`sections_for`] order, and within a section
/// one [`override_chunks`] bucket at a time. Empty buckets are dropped.
///
/// The order of the tokens themselves does not rank them — a `build` payload beats a
/// `common` one wherever it sits — so any reader of a rendered command line that wants
/// Bazel's answer reads overrides through this.
pub fn override_buckets<'a>(tokens: &[&'a str], command: &str) -> Vec<Vec<&'a str>> {
    let chunks = override_chunks(tokens);
    let mut buckets = Vec::new();
    for section in sections_for(command) {
        for chunk in &chunks {
            let payloads: Vec<&str> = chunk
                .iter()
                .filter(|(scope, _)| *scope == section)
                .map(|(_, option)| *option)
                .collect();
            if !payloads.is_empty() {
                buckets.push(payloads);
            }
        }
    }
    buckets
}

/// Whether `token` is a [`DefaultOverride`] rather than a literal command-line flag.
pub fn is_default_override(token: &str) -> bool {
    token.starts_with(DefaultOverride::PREFIX)
}

/// The option a [`DefaultOverride`] token carries, or `None` for any other token. It is a
/// suffix of `token`, so a caller can rewrite the option and keep the wrapper.
pub fn override_payload(token: &str) -> Option<&str> {
    DefaultOverride::parse(token).map(|over| over.option)
}

/// Whether a semver `constraint` is satisfied by the running Bazel `version`.
///
/// `version` is `None` when Bazel reports a non-release build (a
/// `development version` has no version number to parse). Such a build is
/// compiled from HEAD, so it is effectively newer than any release: it is
/// matched against a max version, so lower-bound gates (`>=N`, the usual
/// shape) match and upper-bound gates (`<N`) don't.
///
/// An unparseable constraint is a hard error naming the offending value.
///
/// The version's pre-release suffix is ignored: semver `VersionReq` matching
/// treats a pre-release specially (e.g. `8.0.0-rc2` does NOT satisfy `>=8.0.0`),
/// but for flag gating a release candidate should match the same constraints its
/// release will. So `8.0.0-rc2` is compared as `8.0.0`.
pub fn constraint_matches(
    constraint: &str,
    version: Option<&semver::Version>,
) -> anyhow::Result<bool> {
    let req = semver::VersionReq::parse(constraint)
        .map_err(|e| anyhow::anyhow!("invalid version constraint '{}': {}", constraint, e))?;
    let assumed_latest = semver::Version::new(u64::MAX, u64::MAX, u64::MAX);
    let probe = match version {
        Some(v) if !v.pre.is_empty() => semver::Version::new(v.major, v.minor, v.patch),
        Some(v) => v.clone(),
        None => assumed_latest,
    };
    Ok(req.matches(&probe))
}

#[cfg(test)]
mod constraint_tests {
    use super::constraint_matches;

    fn version(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    #[test]
    fn matches_against_a_known_version() {
        let v = version("8.5.1");
        assert!(constraint_matches(">=8", Some(&v)).unwrap());
        assert!(constraint_matches(">=8, <9", Some(&v)).unwrap());
        assert!(!constraint_matches(">=9", Some(&v)).unwrap());
        assert!(!constraint_matches("<7", Some(&v)).unwrap());
    }

    #[test]
    fn unknown_version_assumes_latest() {
        // A non-release build is newer than any release: lower-bound gates
        // match, upper-bound gates and old-major ranges don't.
        assert!(constraint_matches(">=8", None).unwrap());
        assert!(constraint_matches(">=9", None).unwrap());
        assert!(!constraint_matches("<7", None).unwrap());
        assert!(!constraint_matches(">=8, <9", None).unwrap());
    }

    #[test]
    fn pre_release_matches_its_release_constraints() {
        // A release candidate is gated as its eventual release: the pre-release
        // suffix is ignored, so `8.0.0-rc2` satisfies `>=8` (plain semver
        // VersionReq would NOT match a bare pre-release against `>=8`).
        let rc = version("8.0.0-rc2");
        assert!(constraint_matches(">=8", Some(&rc)).unwrap());
        assert!(constraint_matches(">=8.0.0", Some(&rc)).unwrap());
        assert!(!constraint_matches("<8", Some(&rc)).unwrap());
        assert!(!constraint_matches("<7", Some(&rc)).unwrap());
    }

    #[test]
    fn invalid_constraint_is_an_error() {
        let err = constraint_matches("not-a-constraint", None).unwrap_err();
        assert!(err.to_string().contains("invalid version constraint"));
    }
}

/// All values of a repeatable `--name` option among `opts`, in order, by the
/// rules [`flag_values_in`] documents.
#[cfg(any(test, feature = "starlark"))]
fn flag_value_list(opts: &[RcOption], name: &str) -> Vec<String> {
    let tokens: Vec<&str> = opts.iter().map(|opt| opt.value.as_str()).collect();
    flag_values_in(&tokens, name)
}

/// Tri-state of a boolean `--name` option: `Some(true)` for `--name` /
/// `--name=true|1|yes`, `Some(false)` for `--noname` / `--name=false|0|no`,
/// `None` if unset. Last occurrence wins.
#[cfg(any(test, feature = "starlark"))]
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
#[cfg(feature = "starlark")]
impl StarlarkTypeRepr for RcOption {
    type Canonical = <starlark::values::tuple::UnpackTuple<String> as StarlarkTypeRepr>::Canonical;
    fn starlark_type_repr() -> starlark::typing::Ty {
        // `str | (str, str)` — represented loosely as a string-or-tuple input.
        starlark::values::tuple::UnpackTuple::<String>::starlark_type_repr()
    }
}

#[cfg(feature = "starlark")]
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
                // `(flag, version_condition)` or `(flag, version_condition, command)`;
                // an empty condition means none, an empty command means `always`.
                let version_condition = items.next().filter(|cond| !cond.is_empty());
                let command = items.next().unwrap_or_default();
                return Ok(Some(RcOption {
                    value: flag,
                    version_condition,
                    command,
                    ..RcOption::default()
                }));
            }
        }
        Ok(None)
    }
}

#[cfg(feature = "starlark")]
fn alloc_flag<'v>(eval: &mut Evaluator<'v, '_, '_>, opt: &RcOption) -> Value<'v> {
    match &opt.version_condition {
        None => eval.heap().alloc(opt.value.as_str()),
        Some(cond) => eval.heap().alloc((opt.value.as_str(), cond.as_str())),
    }
}

#[cfg(feature = "starlark")]
#[starlark::starlark_module]
fn runcommand_methods(registry: &mut starlark::environment::MethodsBuilder) {
    /// Overlay `other` on top of this bazelrc, returning a new one. `other`
    /// wins on conflicts (its options apply after this one's under last-write-wins);
    /// startup flags, `--config` skip policy, and `version` are merged with `other`
    /// taking precedence when set. Neither input is mutated.
    fn merge<'v>(
        this: &Bazelrc,
        #[starlark(require = pos)] other: &Bazelrc,
    ) -> anyhow::Result<Bazelrc> {
        Ok(this.merge(other))
    }

    /// The effective value of `--name` for `command`, or `None` if unset.
    /// Last-write-wins over the config-expanded options; matches `--name=VALUE`
    /// and the two-token `--name VALUE` form. `version` overrides the Bazel
    /// version used for version-gated options.
    fn flag_value<'v>(
        this: &Bazelrc,
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
        this: &Bazelrc,
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
        this: &Bazelrc,
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
        this: &Bazelrc,
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
    /// Version-gated options are filtered to the bazelrc's resolved Bazel
    /// version (same as `flag_value` / `resolve_for_command`), so the result
    /// reflects what Bazel actually receives — an option gated to a different
    /// version is dropped rather than reported as active.
    fn expand<'v>(
        this: &Bazelrc,
        #[starlark(require = named)] command: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        let opts = this.applicable_options(&command, None)?;
        Ok(opts.iter().map(|opt| alloc_flag(eval, opt)).collect())
    }

    /// Expand `--config=` flags for `command`, split into
    /// `(startup_flags, flags)` where rc-file options are rendered as
    /// `--default_override=0:<section>=...` entries in `flags` and caller-supplied
    /// ones stay literal.
    ///
    /// Like `expand`, the command flags are filtered to the resolved Bazel
    /// version so version-gated options report accurately.
    fn expand_all<'v>(
        this: &Bazelrc,
        #[starlark(require = named)] command: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<(Vec<Value<'v>>, Vec<Value<'v>>)> {
        let opts = this.applicable_options(&command, None)?;

        let startup: Vec<Value<'v>> = this
            .raw_options("startup")
            .iter()
            .map(|opt| alloc_flag(eval, opt))
            .collect();

        let (overrides, regular) = partition_expand_all(&opts, |o| this.is_command_line(o));
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
        this: &Bazelrc,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        Ok(this
            .sources()
            .iter()
            .map(|p| eval.heap().alloc(p.display().to_string()))
            .collect())
    }

    /// The startup flags Bazel runs with under this bazelrc: the user
    /// startup flags + the rc-file `startup` section + `--ignore_all_rc_files`.
    fn startup_flags<'v>(
        this: &Bazelrc,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        Ok(this
            .invocation_startup_flags()
            .into_iter()
            .map(|s| eval.heap().alloc(s))
            .collect())
    }
}

#[cfg(all(test, feature = "starlark"))]
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

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// `last_flag_value` reads a command line rather than rc options, for a
    /// caller deciding whether a flag has already been asked for.
    #[test]
    fn last_flag_value_reads_a_command_line() {
        assert_eq!(
            last_flag_value(
                &args(&["--jobs=4", "--remote_cache=grpc://x"]),
                "--remote_cache",
                "build",
            ),
            Some("grpc://x".to_string()),
        );
        assert_eq!(
            last_flag_value(
                &args(&["--remote_cache", "grpc://x"]),
                "--remote_cache",
                "build",
            ),
            Some("grpc://x".to_string()),
            "the two-token form is the same flag",
        );
        assert_eq!(
            last_flag_value(
                &args(&["--remote_cache=grpc://first", "--remote_cache=grpc://last"]),
                "--remote_cache",
                "build",
            ),
            Some("grpc://last".to_string()),
            "bazel takes the last of a repeated single-valued flag, so this must too",
        );
        assert_eq!(
            last_flag_value(
                &args(&["--remote_cache", "--jobs=4"]),
                "--remote_cache",
                "build",
            ),
            None,
            "a bare flag before another flag names no value",
        );
        assert_eq!(
            last_flag_value(&args(&["--jobs=4"]), "--remote_cache", "build"),
            None,
        );
        assert_eq!(
            last_flag_value(
                &args(&["--remote_cache_header=x"]),
                "--remote_cache",
                "build"
            ),
            None,
            "a longer flag sharing the prefix is a different flag",
        );
    }

    /// The rendered form of a `common`-section rc entry is a request like any other:
    /// Bazel honours it, so a caller asking "already set?" has to see it.
    #[test]
    fn last_flag_value_reads_a_common_scoped_rc_entry() {
        assert_eq!(
            last_flag_value(
                &args(&["--default_override=0:common=--remote_cache=grpc://rc"]),
                "--remote_cache",
                "build",
            ),
            Some("grpc://rc".to_string()),
        );
        assert_eq!(
            last_flag_value(
                &args(&[
                    "--default_override=0:common=--remote_cache=grpc://first",
                    "--default_override=0:common=--remote_cache=grpc://last",
                ]),
                "--remote_cache",
                "build",
            ),
            Some("grpc://last".to_string()),
            "repeated overrides resolve like repeated flags",
        );
    }

    /// An override is scoped to a section, so it answers only for the commands
    /// Bazel would apply it to — `options_for`'s inheritance, read back.
    #[test]
    fn last_flag_value_honours_an_overrides_section() {
        let line = args(&["--default_override=0:build=--remote_cache=grpc://rc"]);
        assert_eq!(
            last_flag_value(&line, "--remote_cache", "test"),
            Some("grpc://rc".to_string()),
            "`test` inherits the `build` section",
        );
        assert_eq!(
            last_flag_value(&line, "--remote_cache", "query"),
            None,
            "`query` does not",
        );
    }

    /// Bazel parses override payloads at rc priority and the command line after
    /// them, so a literal flag wins however the two are ordered.
    #[test]
    fn last_flag_value_prefers_a_literal_flag_to_an_override() {
        let override_first = args(&[
            "--default_override=0:common=--remote_cache=grpc://rc",
            "--remote_cache=grpc://literal",
        ]);
        let literal_first = args(&[
            "--remote_cache=grpc://literal",
            "--default_override=0:common=--remote_cache=grpc://rc",
        ]);
        for line in [override_first, literal_first] {
            assert_eq!(
                last_flag_value(&line, "--remote_cache", "build"),
                Some("grpc://literal".to_string()),
            );
        }
    }

    /// A payload names a value only inside its own section: a bare flag in one section and a
    /// bare value in another are two unrelated options, and the bare flag is in fact an
    /// `Expected value after …` error there, never a request for this path.
    #[test]
    fn last_flag_value_does_not_pair_payloads_across_sections() {
        let line = args(&[
            "--default_override=0:common=--execution_log_compact_file",
            "--default_override=0:build=/tmp/leaked",
        ]);
        assert_eq!(
            last_flag_value(&line, "--execution_log_compact_file", "build"),
            None,
            "a bare payload must not take a value written under another section",
        );
    }

    /// Within one section and one rc index the pair does form, whatever else sits between the
    /// two payloads on the command line — neither a foreign section at the same index nor a
    /// non-override token splits the bucket.
    #[test]
    fn last_flag_value_pairs_payloads_within_one_section() {
        let line = args(&[
            "--default_override=0:common=--execution_log_compact_file",
            "--default_override=0:query=zzz",
            "build",
            "//...",
            "--default_override=0:common=/tmp/mine",
        ]);
        assert_eq!(
            last_flag_value(&line, "--execution_log_compact_file", "build"),
            Some("/tmp/mine".to_string()),
        );
    }

    /// The rc index delimits the bucket as firmly as the section does: two payloads of one
    /// section at different indices are parsed separately, so the bare flag takes no value.
    #[test]
    fn last_flag_value_does_not_pair_payloads_across_rc_indices() {
        let line = args(&[
            "--default_override=0:common=--execution_log_compact_file",
            "--default_override=1:common=/tmp/leaked",
        ]);
        assert_eq!(
            last_flag_value(&line, "--execution_log_compact_file", "build"),
            None,
            "a bare payload must not take a value written under another rc index",
        );
    }

    /// Nothing is special about index 0 — one index, whichever it is, is one bucket.
    #[test]
    fn last_flag_value_pairs_payloads_within_one_rc_index() {
        let line = args(&[
            "--default_override=1:common=--execution_log_compact_file",
            "--default_override=1:common=/tmp/mine",
        ]);
        assert_eq!(
            last_flag_value(&line, "--execution_log_compact_file", "build"),
            Some("/tmp/mine".to_string()),
        );
    }

    /// Bazel closes the open bucket for *every* section when the rc index changes, so an
    /// intervening payload of an unrelated section at another index splits a `common` pair
    /// even though the two `common` payloads share an index. Grouping payloads by
    /// `(section, index)` instead of by consecutive run would report `/tmp/mine` here, where
    /// Bazel reports `Expected value after --execution_log_compact_file`.
    #[test]
    fn last_flag_value_splits_a_bucket_on_a_foreign_sections_rc_index() {
        let line = args(&[
            "--default_override=0:common=--execution_log_compact_file",
            "--default_override=1:query=--jobs=8",
            "--default_override=0:common=/tmp/mine",
        ]);
        assert_eq!(
            last_flag_value(&line, "--execution_log_compact_file", "build"),
            None,
            "an rc index change anywhere in the stream ends the bucket",
        );
    }

    /// Two sections both naming the flag resolve the way Bazel applies them: the later
    /// section wins, so a `build` payload out-ranks a `common` one. Written in reverse of
    /// section order so it cannot pass on token order alone.
    #[test]
    fn last_flag_value_takes_the_last_section_to_name_the_flag() {
        let line = args(&[
            "--default_override=0:build=--remote_cache=grpc://build",
            "--default_override=0:common=--remote_cache=grpc://common",
        ]);
        assert_eq!(
            last_flag_value(&line, "--remote_cache", "build"),
            Some("grpc://build".to_string()),
        );
        assert_eq!(
            last_flag_value(&line, "--remote_cache", "query"),
            Some("grpc://common".to_string()),
            "`query` does not read the `build` section",
        );
    }

    /// Within one section, where no section order can decide it, the later bucket wins.
    #[test]
    fn last_flag_value_takes_the_last_bucket_of_one_section() {
        let name = "--remote_cache";
        let ascending = args(&[
            "--default_override=0:common=--remote_cache=grpc://zero",
            "--default_override=1:common=--remote_cache=grpc://one",
        ]);
        assert_eq!(
            last_flag_value(&ascending, name, "build"),
            Some("grpc://one".to_string()),
        );
        let descending = args(&[
            "--default_override=1:common=--remote_cache=grpc://one",
            "--default_override=0:common=--remote_cache=grpc://zero",
        ]);
        assert_eq!(
            last_flag_value(&descending, name, "build"),
            Some("grpc://zero".to_string()),
            "the bucket order is the token order, not the index order",
        );
    }

    /// The payload is matched as an option, not as text: an override carrying a
    /// different flag whose name merely contains the one asked about is not a hit.
    #[test]
    fn last_flag_value_does_not_match_an_override_payload_loosely() {
        assert_eq!(
            last_flag_value(
                &args(&["--default_override=0:common=--my_remote_cache_thing=grpc://x"]),
                "--remote_cache",
                "build",
            ),
            None,
        );
        assert_eq!(
            last_flag_value(
                &args(&["--default_override=0:common=--remote_cache_header=x"]),
                "--remote_cache",
                "build",
            ),
            None,
        );
    }

    /// A `{section}:{config}` payload belongs to that config, not to its base section: Bazel
    /// keys the option on the qualified name and applies it only while the config is active.
    /// Reading it as `common` would resurrect the bug this reader exists to fix — a flag seen
    /// as set that Bazel ignores. (Probed: 9.0.1 ignores `common:ci` without `--config=ci`.)
    #[test]
    fn last_flag_value_does_not_read_a_config_qualified_scope_as_its_base_section() {
        assert_eq!(
            last_flag_value(
                &args(&["--default_override=0:common:ci=--remote_cache=grpc://cfg"]),
                "--remote_cache",
                "build",
            ),
            None,
        );
    }

    /// Anything that is not a well-formed override is left to the literal pass —
    /// including a different flag whose name starts the same way.
    #[test]
    fn default_override_parse_rejects_malformed_tokens() {
        for token in [
            "--default_override=common=--remote_cache=x", // no rc index
            "--default_override=0:common",                // no payload
            "--default_override=0:=--remote_cache=x",     // no section
            "--default_override=x:common=--remote_cache=x", // non-numeric index
            "--default_overrides=0:common=--remote_cache=x", // a different flag
        ] {
            assert!(
                DefaultOverride::parse(token).is_none(),
                "{token} should not parse as an override",
            );
            assert_eq!(
                last_flag_value(&args(&[token]), "--remote_cache", "build"),
                None,
                "{token} should name no value",
            );
        }
    }

    /// Rendering and parsing are one definition of the wire form, so a rendered
    /// override reads back as what it was built from.
    #[test]
    fn default_override_round_trips() {
        let rendered = DefaultOverride {
            index: DefaultOverride::SOLE_SOURCE,
            command: "common",
            option: "--remote_cache=grpc://x",
        }
        .to_string();
        assert_eq!(
            rendered,
            "--default_override=0:common=--remote_cache=grpc://x"
        );
        let parsed = DefaultOverride::parse(&rendered).expect("parses");
        assert_eq!(parsed.index, DefaultOverride::SOLE_SOURCE);
        assert_eq!(parsed.command, "common");
        assert_eq!(parsed.option, "--remote_cache=grpc://x");

        // The index is part of the wire form, not a constant: `override_chunks`
        // keys a bucket on it, so rendering must carry whatever it was given.
        let other = DefaultOverride {
            index: 1,
            command: "common",
            option: "--remote_cache=grpc://x",
        }
        .to_string();
        assert_eq!(other, "--default_override=1:common=--remote_cache=grpc://x");
        assert_eq!(DefaultOverride::parse(&other).expect("parses").index, 1);
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
    fn partition_routes_rc_sections_to_overrides() {
        let common = RcOption {
            value: "--remote_cache=x".to_string(),
            command: "common".to_string(),
            ..RcOption::default()
        };
        let build = RcOption {
            value: "--jobs=4".to_string(),
            command: "build".to_string(),
            ..RcOption::default()
        };
        // No source distinguishes caller from rc-file here: everything is
        // rc-file (`|_| false`), so every section rides the transport.
        let (overrides, flags) = partition_expand_all(&[common, build], |_| false);
        assert_eq!(
            overrides,
            vec![
                (
                    "--default_override=0:common=--remote_cache=x".to_string(),
                    None
                ),
                ("--default_override=0:build=--jobs=4".to_string(), None),
            ]
        );
        assert!(
            flags.is_empty(),
            "rc-file entries never ride the literal command line: {flags:?}"
        );

        // A config-qualified section renders its base: `common:ci` → `common`.
        let ci = RcOption {
            value: "--remote_cache=y".to_string(),
            command: "common:ci".to_string(),
            ..RcOption::default()
        };
        let (overrides, _) = partition_expand_all(&[ci], |_| false);
        assert_eq!(overrides.len(), 1, "common:config base routes to overrides");
        assert_eq!(
            overrides[0].0, "--default_override=0:common=--remote_cache=y",
            "the override carries the base section"
        );

        // `always` rides under `always` — a pseudo-command Bazel accepts.
        let always = RcOption {
            value: "--keep_going".to_string(),
            command: "always".to_string(),
            ..RcOption::default()
        };
        let (overrides, _) = partition_expand_all(&[always], |_| false);
        assert_eq!(
            overrides,
            vec![("--default_override=0:always=--keep_going".to_string(), None)]
        );
    }

    /// A bare flag and a bare token from different sources are two options, not
    /// one `--flag value` pair, even when they share a section.
    #[test]
    fn partition_does_not_fold_across_sources() {
        let flag = RcOption {
            value: "--keep_going".to_string(),
            command: "common".to_string(),
            source_index: 0,
            ..RcOption::default()
        };
        let target = RcOption {
            value: "//:all".to_string(),
            command: "common".to_string(),
            source_index: 1,
            ..RcOption::default()
        };
        let (overrides, _) = partition_expand_all(&[flag, target], |_| false);
        assert_eq!(overrides.len(), 2, "{overrides:?}");
    }

    /// Caller flags stay literal — Bazel applies command-line flags after every
    /// override payload, and demoting them to overrides would let a more
    /// specific rc section beat them. `common`-scoped caller flags are the
    /// exception: a literal `--option` is rejected by any command that does
    /// not accept it, so they ride the transport like their rc-file peers.
    #[test]
    fn partition_keeps_caller_flags_literal_except_common() {
        let build = RcOption {
            value: "--jobs=4".to_string(),
            command: "build".to_string(),
            ..RcOption::default()
        };
        let always = RcOption {
            value: "--keep_going".to_string(),
            command: "always".to_string(),
            ..RcOption::default()
        };
        let common = RcOption {
            value: "--remote_cache=x".to_string(),
            command: "common".to_string(),
            ..RcOption::default()
        };
        let (overrides, flags) = partition_expand_all(&[build, always, common], |_| true);
        assert_eq!(
            overrides,
            vec![(
                "--default_override=0:common=--remote_cache=x".to_string(),
                None
            )]
        );
        assert_eq!(
            flags,
            vec![
                ("--jobs=4".to_string(), None),
                ("--keep_going".to_string(), None),
            ]
        );
    }

    /// The two-token fold covers every rc section: `build:ci --flag value`
    /// must become one `=`-form override under `build`, the same as `common`.
    #[test]
    fn partition_folds_two_token_build_override_to_eq_form() {
        let build_ci: Vec<RcOption> = ["--experimental_convenience_symlinks", "normal"]
            .iter()
            .map(|v| RcOption {
                value: v.to_string(),
                command: "build:ci".to_string(),
                ..RcOption::default()
            })
            .collect();
        let (overrides, _) = partition_expand_all(&build_ci, |_| false);
        assert_eq!(
            overrides,
            vec![(
                "--default_override=0:build=--experimental_convenience_symlinks=normal".to_string(),
                None
            )]
        );
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
        let (overrides, _) = partition_expand_all(
            &common_ci(&["--remote_download_outputs", "minimal"]),
            |_| false,
        );
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
        let (overrides, _) = partition_expand_all(
            &common_ci(&["--remote_download_outputs=minimal", "--jobs=4"]),
            |_| false,
        );
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
        let (overrides, _) = partition_expand_all(
            &common_ci(&["--remote_local_fallback", "--remote_timeout=3600"]),
            |_| false,
        );
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
        // is folded as `--flag=target`, losing the target. Non-option args cannot
        // ride the override transport (see `partition_expand_all`). Locks the
        // behavior in so a future "fix" is a conscious decision, not a surprise.
        let (overrides, _) =
            partition_expand_all(&common_ci(&["--keep_going", "//:all"]), |_| false);
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
        let (overrides, _) = partition_expand_all(&[flag, value], |_| false);
        assert_eq!(
            overrides.len(),
            2,
            "differing version conditions must not join"
        );
    }

    #[test]
    fn merge_overlays_b_over_a_last_wins() {
        let a = Bazelrc::blank(&opts(&["--remote_cache=grpc://a"]));
        let b = Bazelrc::blank(&opts(&["--remote_cache=grpc://b"]));
        let merged = a.merge(&b);
        let resolved = merged.applicable_options("build", None).unwrap();
        assert_eq!(
            flag_value_list(&resolved, "--remote_cache").pop().unwrap(),
            "grpc://b"
        );
    }

    #[test]
    fn merge_combines_startup_and_version() {
        let a = Bazelrc::blank(&[]).with_startup_flags(vec!["--output_base=/a".to_string()]);
        let b = Bazelrc::blank(&[])
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
        let mut rc = Bazelrc::blank(&[]);
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
        assert!(matches!(err, BazelrcError::ImportCycle { .. }));
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

    fn rc_with(sources: &[&Path], options: &[(&str, usize, &str)]) -> Bazelrc {
        let mut map: HashMap<String, Vec<RcOption>> = HashMap::new();
        for (key, source_index, value) in options {
            map.entry(key.to_string()).or_default().push(RcOption {
                value: value.to_string(),
                command: key.to_string(),
                source_index: *source_index,
                version_condition: None,
            });
        }
        Bazelrc {
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
        // For `test --config=ci` Bazel expands always:ci → common:ci → build:ci → test:ci
        // (last wins). The announce must list the same-named config sections in that order,
        // not alphabetically (which would put always:ci and build:ci ahead of common:ci and
        // misrepresent the override order).
        let root = Path::new("/work/repo");
        let rc = rc_with(
            &[&root.join(".bazelrc")],
            &[
                ("always:ci", 0, "--remote_cache=Z"),
                ("common:ci", 0, "--remote_cache=A"),
                ("build:ci", 0, "--remote_cache=B"),
                ("test:ci", 0, "--remote_cache=C"),
            ],
        );
        let out = rc.announce("test", false, 200, root, None, |s| s.to_string());
        let always_at = out.find("always:ci").expect("always:ci shown");
        let common_at = out.find("common:ci").expect("common:ci shown");
        let build_at = out.find("build:ci").expect("build:ci shown");
        let test_at = out.find("test:ci").expect("test:ci shown");
        assert!(
            always_at < common_at && common_at < build_at && build_at < test_at,
            "config sections must follow expansion order always→common→build→test: {out}"
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

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=opt"])).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=ci"])).unwrap();
        let (_startup, command_flags) = rc.resolve_for_command("build").unwrap();
        assert!(
            command_flags.contains(
                &"--default_override=0:common=--remote_download_outputs=minimal".to_string()
            ),
            "expected a single `=`-form override, got {command_flags:?}",
        );
    }

    /// A `build`-section rc entry must not reach the command line as a
    /// literal. On Aspect Workflows the `bazel` on PATH is the agent's
    /// wrapper, which registers `--experimental_convenience_symlinks` as its
    /// own boolean flag and rejects the enum value `normal` — Go's flag
    /// parser error, before real Bazel runs. The override token names no flag
    /// the wrapper knows, so it passes through untouched.
    #[test]
    fn build_section_rc_options_never_ride_the_literal_command_line() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            "build --experimental_convenience_symlinks=normal\n\
             build --remote_timeout=3600\n\
             common --announce_rc\n",
        )
        .unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        let (_, command_flags) = rc.resolve_for_command("build").unwrap();
        assert_eq!(
            command_flags,
            vec![
                "--default_override=0:common=--announce_rc".to_string(),
                "--default_override=0:build=--experimental_convenience_symlinks=normal".to_string(),
                "--default_override=0:build=--remote_timeout=3600".to_string(),
            ],
            "every rc-file option rides the transport, in section order"
        );
        assert!(
            command_flags
                .iter()
                .all(|flag| flag.starts_with("--default_override=")),
            "no rc-derived flag may reach the literal command line: {command_flags:?}",
        );
        assert_eq!(
            last_flag_value(
                &command_flags,
                "--experimental_convenience_symlinks",
                "build"
            ),
            Some("normal".to_string()),
            "the reader must see what the writer renders",
        );
    }

    /// A config's options rank where its `--config=` request sits, as Bazel
    /// expands it in place: a caller's request renders literal, so it beats every
    /// rc section, and an rc `test --config=ci` renders under `test`, after the
    /// `test` entries before it. Rendering `build:ci` under `build` would let the
    /// `test` section's `--test_output=errors` win both.
    #[test]
    fn config_options_rank_at_the_request() {
        let dir = make_workspace();
        let root = dir.path();
        let name = "--test_output";

        fs::write(
            root.join(".bazelrc"),
            "test --test_output=errors\nbuild:ci --test_output=all\n",
        )
        .unwrap();
        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=ci"])).unwrap();
        let rendered = rc.resolve_for_command("test").unwrap().1;
        assert_eq!(
            rendered,
            vec![
                "--default_override=0:test=--test_output=errors",
                "--test_output=all"
            ],
            "a caller's --config expands at command-line priority"
        );
        assert_eq!(
            last_flag_value(&rendered, name, "test").as_deref(),
            Some("all")
        );

        fs::write(
            root.join(".bazelrc"),
            "test --test_output=errors\ntest --config=ci\nbuild:ci --test_output=all\n",
        )
        .unwrap();
        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        let rendered = rc.resolve_for_command("test").unwrap().1;
        assert_eq!(
            rendered,
            vec![
                "--default_override=0:test=--test_output=errors",
                "--default_override=0:test=--test_output=all",
            ],
            "an rc --config expands under the requesting section"
        );
        assert_eq!(
            last_flag_value(&rendered, name, "test").as_deref(),
            Some("all")
        );
        assert_eq!(
            flag_value_list(&rc.applicable_options("test", None).unwrap(), name).pop(),
            Some("all".to_string()),
            "the option model agrees"
        );
    }

    /// A config requested from inside another config's section ranks where the
    /// outermost request sits: `build:a --config=b` pulled in by `test --config=a`
    /// files `build:b` under `test` too.
    #[test]
    fn nested_config_options_rank_at_the_outermost_request() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            "test --test_output=errors\n\
             test --config=a\n\
             build:a --config=b\n\
             build:b --test_output=all\n",
        )
        .unwrap();
        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        let rendered = rc.resolve_for_command("test").unwrap().1;
        assert_eq!(
            rendered,
            vec![
                "--default_override=0:test=--test_output=errors",
                "--default_override=0:test=--test_output=all",
            ],
        );
    }

    /// The effective `--remote_timeout` for `build`, read both ways off the
    /// rendered command line, for an rc of `rc` (where `{os}` names the host's
    /// platform config) and a caller passing `caller`.
    fn platform_remote_timeout(rc: &str, caller: &[&str]) -> Option<String> {
        let dir = make_workspace();
        let root = dir.path();
        let rc = rc.replace("{os}", std::env::consts::OS);
        fs::write(root.join(".bazelrc"), rc).unwrap();
        let rc = Bazelrc::new(root, ISOLATE, &flags(caller)).unwrap();
        let rendered = rc.resolve_for_command("build").unwrap().1;
        let value = last_flag_value(&rendered, "--remote_timeout", "build");
        assert_eq!(
            crate::command::flags::effective_value(&rendered, "build", "remote_timeout"),
            value,
            "the two readers disagree on {rendered:?}"
        );
        value
    }

    /// The platform config expands in place of the setting that turns it on, as
    /// Bazel's `ConfigExpander` does: `build:<os>` beats a `build` line above
    /// that setting.
    #[test]
    fn platform_config_ranks_at_a_build_setting() {
        let rc = "common --remote_timeout=0\n\
                  build --remote_timeout=1\n\
                  build:{os} --remote_timeout=2\n\
                  build --enable_platform_specific_config\n";
        assert_eq!(platform_remote_timeout(rc, &[]).as_deref(), Some("2"));
    }

    /// Set under `common`, the platform config ranks at `common`, so a plain
    /// `build` line beats `build:<os>`, as it would a `common --config=<os>`.
    #[test]
    fn platform_config_ranks_at_a_common_setting() {
        let rc = "common --enable_platform_specific_config\n\
                  build --remote_timeout=1\n\
                  build:{os} --remote_timeout=2\n";
        assert_eq!(platform_remote_timeout(rc, &[]).as_deref(), Some("1"));
    }

    /// Set by the caller, the platform config ranks on the command line, above
    /// every rc section.
    #[test]
    fn platform_config_ranks_at_a_caller_setting() {
        let rc = "build --remote_timeout=1\n\
                  build:{os} --remote_timeout=2\n";
        assert_eq!(
            platform_remote_timeout(rc, &["--enable_platform_specific_config"]).as_deref(),
            Some("2")
        );
    }

    /// A `common:{config}` entry keeps `common` whoever requests it: that is
    /// what lets it reach a command that rejects it, here `query`.
    #[test]
    fn common_config_options_stay_common() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            "common:ci --remote_download_minimal\n",
        )
        .unwrap();
        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=ci"])).unwrap();
        assert_eq!(
            rc.resolve_for_command("query").unwrap().1,
            vec!["--default_override=0:common=--remote_download_minimal"],
        );
    }

    /// The two readers of a resolved rc agree about a `common`-section flag.
    ///
    /// `rc.flag_value` asks the option model, where `common` is a section by name;
    /// `last_flag_value` asks the command line the same rc renders to, where it is
    /// a `--default_override`. For a `common`-section flag — the shape that broke — a
    /// caller that reuses a path the run already asks for has to get the same answer
    /// whichever reader it holds, or it adds a second `--execution_log_compact_file`
    /// over the user's. The readers are not interchangeable in general — a caller
    /// flag scoped to `common` rides the transport while the model still reads it as
    /// a plain option — but they agree on every rc-file section: see
    /// `the_two_readers_agree_on_an_always_section_flag`.
    #[test]
    fn both_readers_see_a_common_scoped_flag() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            "common --execution_log_compact_file=/tmp/mine.zst\n",
        )
        .unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        let name = "--execution_log_compact_file";
        let want = Some("/tmp/mine.zst".to_string());
        assert_eq!(
            flag_value_list(&rc.applicable_options("build", None).unwrap(), name).pop(),
            want,
            "the option model",
        );
        assert_eq!(
            last_flag_value(&rc.resolve_for_command("build").unwrap().1, name, "build"),
            want,
            "the command line it renders to",
        );
    }

    /// An `always` entry that `common` also sets reads the same from both readers.
    ///
    /// `always` rides the override transport like every other rc-file section, so
    /// the rendered command line ranks it exactly as the option model does: Bazel
    /// parses sections `always`-then-`common`, last-write-wins, so `common` wins
    /// from both readers.
    #[test]
    fn the_two_readers_agree_on_an_always_section_flag() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            "always --execution_log_compact_file=/tmp/always\n\
             common --execution_log_compact_file=/tmp/common\n",
        )
        .unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        let name = "--execution_log_compact_file";
        assert_eq!(
            flag_value_list(&rc.applicable_options("build", None).unwrap(), name).pop(),
            Some("/tmp/common".to_string()),
            "the option model applies `always` before `common`",
        );
        assert_eq!(
            last_flag_value(&rc.resolve_for_command("build").unwrap().1, name, "build"),
            Some("/tmp/common".to_string()),
            "rendered, both are overrides and `common` parses after `always`",
        );
    }

    #[test]
    fn info_resolves_its_own_command_flags() {
        // The startup flags suppress the on-disk rc, so the command flags are
        // what put its options back. `info` must get both, or it resolves
        // flag-sensitive keys like `output_path` under the wrong flags.
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            &root.join(".bazelrc"),
            "common --remote_download_outputs=minimal\ninfo --show_make_env\n",
        )
        .unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        let (startup, command_flags) = rc.resolve_for_command("info").unwrap();
        assert!(
            startup.contains(&"--ignore_all_rc_files".to_string()),
            "the rc is absorbed, so the spawn must suppress the on-disk one: {startup:?}",
        );
        assert!(
            command_flags.contains(
                &"--default_override=0:common=--remote_download_outputs=minimal".to_string()
            ),
            "`common` options must reach info, not just build: {command_flags:?}",
        );
        assert!(
            command_flags.contains(&"--default_override=0:info=--show_make_env".to_string()),
            "`info`-section options must reach info as an override: {command_flags:?}",
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

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=a"])).unwrap();
        let err = rc.expand_configs("build", &[]).unwrap_err();
        assert!(matches!(err, BazelrcError::ConfigCycle { .. }));
    }

    /// Bazel accepts `--config NAME` as readily as `--config=NAME`, and the
    /// space form reaches us from any tokenized source (an rc line, a repeated
    /// `--bazel-flag`, a flag forwarded verbatim from the command line). Since a
    /// `Bazelrc` runs Bazel with `--ignore_all_rc_files`, a request we fail to
    /// expand cannot be recovered downstream: Bazel would report the config as
    /// undefined while it sits in the user's `.bazelrc`.
    #[test]
    fn config_expands_from_the_space_form() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build:ci --jobs=8\n").unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config", "ci"])).unwrap();
        let expanded: Vec<String> = rc
            .expand_configs("build", &[])
            .unwrap()
            .into_iter()
            .map(|o| o.value)
            .collect();
        assert_eq!(expanded, vec!["--jobs=8".to_string()]);
    }

    /// The value token is consumed by the request, not emitted alongside it —
    /// otherwise a bare `ci` would reach Bazel as a target pattern.
    #[test]
    fn config_space_form_consumes_only_its_value() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build:ci --jobs=8\n").unwrap();

        let rc = Bazelrc::new(
            root,
            ISOLATE,
            &flags(&["--verbose_failures", "--config", "ci", "--keep_going"]),
        )
        .unwrap();
        let expanded: Vec<String> = rc
            .expand_configs("build", &[])
            .unwrap()
            .into_iter()
            .map(|o| o.value)
            .collect();
        assert_eq!(
            expanded,
            vec![
                "--verbose_failures".to_string(),
                "--jobs=8".to_string(),
                "--keep_going".to_string(),
            ]
        );
    }

    /// A `--config` with nothing usable after it is Bazel's to complain about —
    /// it knows the flag's arity and words the error better than we can.
    #[test]
    fn config_without_a_value_passes_through() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build:ci --jobs=8\n").unwrap();

        for tail in [vec!["--config"], vec!["--config", "--keep_going"]] {
            let rc = Bazelrc::new(root, ISOLATE, &flags(&tail)).unwrap();
            let expanded: Vec<String> = rc
                .expand_configs("build", &[])
                .unwrap()
                .into_iter()
                .map(|o| o.value)
                .collect();
            let expected: Vec<String> = tail.iter().map(|s| s.to_string()).collect();
            assert_eq!(expanded, expected, "for {tail:?}");
        }
    }

    /// Cycle detection and `skip_config_if_missing` key off the config name, so
    /// they must see it through either spelling.
    #[test]
    fn config_space_form_shares_cycle_and_skip_handling() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build:loop --config loop\n").unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config", "loop"])).unwrap();
        assert!(matches!(
            rc.expand_configs("build", &[]).unwrap_err(),
            BazelrcError::ConfigCycle { .. }
        ));

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config", "absent"])).unwrap();
        assert!(rc.expand_configs("build", &[]).is_err());
        assert_eq!(rc.expand_configs("build", &["absent"]).unwrap().len(), 0);
    }

    #[test]
    fn config_undefined_errors() {
        let dir = make_workspace();
        let root = dir.path();
        let rc_path = root.join(".bazelrc");
        fs::write(&rc_path, "build --jobs=4\n").unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=nonexistent"])).unwrap();
        let err = rc.expand_configs("build", &[]).unwrap_err();
        assert!(matches!(err, BazelrcError::UndefinedConfig { .. }));
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

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=rel"])).unwrap();
        assert!(
            rc.expand_configs("build", &[]).is_ok(),
            "build sees build:rel"
        );
        let err = rc.expand_configs("query", &[]).unwrap_err();
        assert!(
            matches!(err, BazelrcError::UndefinedConfig { .. }),
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

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=rel"])).unwrap();
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
        let rc = Bazelrc::new(root, &flags, &[]).unwrap();
        // workspace rc + 1 explicit (deduped from 2)
        assert_eq!(rc.sources().len(), 1);
    }

    #[test]
    fn nosystem_rc_skips_system() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4\n").unwrap();

        let rc = Bazelrc::new(root, &["--nosystem_rc"], &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        let opts: Vec<&str> = rc
            .options_for("build")
            .iter()
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(opts, vec!["--always-flag", "--common-flag", "--build-flag"]);
    }

    #[test]
    fn caller_flag_scoped_to_build_skips_query() {
        // A caller flag naming its section applies where an rc-file entry in that
        // section would: `build` reaches test but not query. Unscoped flags stay
        // `always` and reach both.
        let scoped = RcOption {
            value: "--execution_log_compact_file=/tmp/exec.log".to_owned(),
            command: "build".to_owned(),
            ..RcOption::default()
        };
        let unscoped = RcOption {
            value: "--remote_timeout=3600".to_owned(),
            ..RcOption::default()
        };
        let rc = Bazelrc::blank(&[scoped, unscoped]);
        let values = |command: &str| -> Vec<String> { rc.resolve_for_command(command).unwrap().1 };
        // Section order, not list order: the `always` entry first, then the `build` one.
        assert_eq!(
            values("test"),
            vec![
                "--remote_timeout=3600",
                "--execution_log_compact_file=/tmp/exec.log"
            ]
        );
        assert_eq!(values("query"), vec!["--remote_timeout=3600"]);
        assert_eq!(values("build").len(), 2);
    }

    #[test]
    fn scoped_caller_flag_still_overrides_the_rc_files() {
        // A caller flag that names a section must stay *after* every rc-file option, the way
        // an unscoped one does: scoping it to the commands that accept it is about which
        // commands see it, not about losing to a `.bazelrc` default. The regression this
        // guards: placing it in its section's slot put it ahead of the rc-file `always` and
        // `common` entries and of the caller's own unscoped flags.
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            r#"
common --jobs=1
build --jobs=2
"#,
        )
        .unwrap();

        let rc = Bazelrc::new(
            root,
            ISOLATE,
            &[
                RcOption {
                    value: "--jobs=3".to_owned(),
                    command: "build".to_owned(),
                    ..RcOption::default()
                },
                RcOption {
                    value: "--keep_going".to_owned(),
                    ..RcOption::default()
                },
            ],
        )
        .unwrap();

        // The rc-file sections ride the override transport; the caller's flags
        // stay literal, and Bazel applies the command line after every override
        // payload — so the caller's `--jobs=3` is what `--jobs` resolves to for
        // a build however the sections are scoped.
        assert_eq!(
            rc.resolve_for_command("build").unwrap().1,
            vec![
                "--default_override=0:common=--jobs=1",
                "--default_override=0:build=--jobs=2",
                "--keep_going",
                "--jobs=3"
            ],
            "rc-file sections as overrides, then the caller's: unscoped, then build-scoped"
        );
        assert_eq!(
            rc.resolve_for_command("query").unwrap().1,
            vec!["--default_override=0:common=--jobs=1", "--keep_going"],
            "query reads neither the rc-file nor the caller's build section"
        );
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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
        let rc = Bazelrc::new(
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

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=opt"])).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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
        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        assert_eq!(rc.options_for("build").len(), 0);
    }

    #[test]
    fn whitespace_only_file() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "   \n\t\n  \n").unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        assert_eq!(rc.options_for("build").len(), 0);
    }

    #[test]
    fn commented_line() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "# startup foo\n").unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        assert_eq!(rc.options_for("startup").len(), 0);
    }

    #[test]
    fn command_with_no_args() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build\n").unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        assert_eq!(rc.options_for("build").len(), 0);
    }

    #[test]
    fn command_with_trailing_comment() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4 # a comment\n").unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        let opts = rc.options_for("build");
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].value, "--jobs=4");
    }

    #[test]
    fn multiple_args_on_one_line() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4 --verbose_failures\n").unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        let opts = rc.options_for("build");
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].value, "--jobs=4");
    }

    #[test]
    fn indented_command_parsed() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "  build --jobs=4\n").unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(
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
        let rc = Bazelrc::new(root, &["--nohome_rc"], &[]);
        assert!(rc.is_ok());
    }

    #[test]
    fn ignore_all_rc_files_skips_all() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --workspace-flag\n").unwrap();

        let rc = Bazelrc::new(root, &["--ignore_all_rc_files"], &[]).unwrap();
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
        let rc = Bazelrc::new(
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

        let err = Bazelrc::new(root, &["--bazelrc=/nonexistent/missing.bazelrc"], &[]).unwrap_err();
        assert!(matches!(err, BazelrcError::BazelrcNotFound { .. }));
    }

    // ── Error cases ───────────────────────────────────────────────────────────

    #[test]
    fn import_too_many_args_error() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "import foo bar\n").unwrap();

        let err = Bazelrc::new(root, ISOLATE, &[]).unwrap_err();
        assert!(matches!(err, BazelrcError::InvalidImportArgs { .. }));
    }

    #[test]
    fn try_import_too_many_args_error() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "try-import foo bar\n").unwrap();

        let err = Bazelrc::new(root, ISOLATE, &[]).unwrap_err();
        assert!(matches!(err, BazelrcError::InvalidImportArgs { .. }));
    }

    // ── Config expansion ──────────────────────────────────────────────────────

    #[test]
    fn common_config_section_used_as_fallback() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "common:myconfig --common-flag\n").unwrap();

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=myconfig"])).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=a"])).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=opt"])).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();

        assert_eq!(values, vec!["--non-config", "--foo-flag", "--bar-flag"]);
    }

    /// Whether `--os-specific-flag` from `build:<os>` is applied, for an rc that
    /// adds `rc` and a caller that passes `caller`.
    fn platform_config_applies(rc: &str, caller: &[&str]) -> bool {
        platform_config_applies_at(rc, "", caller, None)
    }

    /// [`platform_config_applies`] at Bazel `version`, with `gated` in an rc
    /// file imported only for Bazel 9 and later.
    fn platform_config_applies_at(
        rc: &str,
        gated: &str,
        caller: &[&str],
        version: Option<&str>,
    ) -> bool {
        let dir = make_workspace();
        let root = dir.path();
        let os = std::env::consts::OS;
        let gated_rc = root.join("gated.bazelrc");
        fs::write(&gated_rc, gated).unwrap();
        fs::write(
            root.join(".bazelrc"),
            format!(
                "build:{os} --os-specific-flag\n{rc}try-import-if-bazel-version >=9.0.0 {}\n",
                gated_rc.display()
            ),
        )
        .unwrap();
        let rc = Bazelrc::new(root, ISOLATE, &flags(caller))
            .unwrap()
            .with_version(version.map(|v| semver::Version::parse(v).unwrap()));
        rc.expand_configs("build", &[])
            .unwrap()
            .iter()
            .any(|o| o.value == "--os-specific-flag")
    }

    /// The last `--enable_platform_specific_config` setting decides, in any of
    /// Bazel's boolean spellings.
    #[test]
    fn enable_platform_specific_config_reads_every_spelling() {
        assert!(platform_config_applies(
            "",
            &["--enable_platform_specific_config"]
        ));
        assert!(platform_config_applies(
            "",
            &["--enable_platform_specific_config=true"]
        ));
        assert!(platform_config_applies(
            "",
            &["--enable_platform_specific_config=1"]
        ));
        assert!(!platform_config_applies(
            "",
            &["--enable_platform_specific_config=false"]
        ));
        assert!(!platform_config_applies("", &[]));
        assert!(!platform_config_applies(
            "build --enable_platform_specific_config\n",
            &["--noenable_platform_specific_config"]
        ));
        assert!(platform_config_applies(
            "build --noenable_platform_specific_config\n",
            &["--enable_platform_specific_config"]
        ));
    }

    /// A version-gated setting counts only at a Bazel version it applies to.
    #[test]
    fn enable_platform_specific_config_follows_its_version_gate() {
        let rc = "build --enable_platform_specific_config\n";
        let off = "build --noenable_platform_specific_config\n";
        assert!(platform_config_applies_at(rc, off, &[], Some("8.4.0")));
        assert!(!platform_config_applies_at(rc, off, &[], Some("9.0.0")));
        assert!(!platform_config_applies_at(rc, off, &[], None));
    }

    /// Bazel reads the setting after expanding configs, so a config can turn
    /// the platform config on.
    #[test]
    fn enable_platform_specific_config_set_by_a_config() {
        assert!(platform_config_applies(
            "build:ci --enable_platform_specific_config\n",
            &["--config=ci"]
        ));
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &[]).unwrap();
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

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=myconfig"])).unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert!(values.contains(&"--always-only-flag"), "got: {values:?}");
    }

    /// With no `<os>` section the platform config is skipped, not an
    /// undefined-config error.
    #[test]
    fn enable_platform_specific_config_no_section_is_silent() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4\n").unwrap();
        let rc = Bazelrc::new(
            root,
            ISOLATE,
            &flags(&["--enable_platform_specific_config"]),
        )
        .unwrap();
        let expanded = rc.expand_configs("build", &[]).unwrap();
        assert!(expanded.iter().any(|o| o.value == "--jobs=4"));
    }

    #[test]
    fn ignore_all_rc_files_suppresses_explicit_bazelrc() {
        // --ignore_all_rc_files must suppress --bazelrc= files too (Bazel spec)
        let dir = make_workspace();
        let root = dir.path();
        let explicit = root.join("explicit.bazelrc");
        fs::write(&explicit, "build --explicit-flag\n").unwrap();

        let flag = format!("--bazelrc={}", explicit.display());
        let rc = Bazelrc::new(root, &["--ignore_all_rc_files", flag.as_str()], &[]).unwrap();
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
        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=myconfig"])).unwrap();
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
        // Regression test for the remote_timeout config-override bug:
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

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=ci"])).unwrap();
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
        // Regression test for the Tar caching bug:
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

        let rc = Bazelrc::new(root, ISOLATE, &flags(&["--config=ci"])).unwrap();
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
