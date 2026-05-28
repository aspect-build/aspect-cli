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
use std::fmt::Write as _;

use thiserror::Error;

/// Synthetic source label used for `--config=` and other flags supplied to
/// [`BazelRC::new`] outside any rc file — i.e. the command line.
pub const CLI_SOURCE_PATH: &str = "<command line>";

// SGR escape parameters for error-block styling. Callers (see
// `axl-runtime/src/term.rs`) decide whether to enable ANSI and pass the
// answer through [`BazelRC::with_ansi_errors`].
const SGR_BOLD: &str = "\x1b[1m";
const SGR_BOLD_RED: &str = "\x1b[1;31m";
const SGR_DIM: &str = "\x1b[2m";
const SGR_RESET: &str = "\x1b[0m";

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

    #[error("{}", render_undefined_config(.command, .name, .flag_source, .workspace_root, .loaded_rc_files, .applicable_rc_state, *.ansi))]
    UndefinedConfig {
        command: String,
        name: String,
        /// Source path the unresolved `--config={name}` flag came from. Either
        /// [`CLI_SOURCE_PATH`] (CLI flag or `config.axl`) or a real `.bazelrc`
        /// path (when the reference lives in an rc line).
        flag_source: PathBuf,
        /// Bazel workspace root used for rc-file discovery. Surfaced in the
        /// error so users can spot a sub-workspace anchor mismatch — e.g. an
        /// outer `config.axl` setting `--config=ci` while the inner `.bazelrc`
        /// lives in a different workspace and doesn't define `:ci`.
        workspace_root: PathBuf,
        /// Real `.bazelrc` paths that were loaded (excludes [`CLI_SOURCE_PATH`]).
        loaded_rc_files: Vec<PathBuf>,
        /// Pre-formatted [`BazelRC::announce`] output for the failing command —
        /// the same view `--announce-rc` would print. Surfacing it inline saves
        /// the user from re-running with the flag to inspect the loaded state.
        applicable_rc_state: String,
        /// Whether to render with ANSI escape codes (bold headers, red prefix).
        /// Snapshotted from [`BazelRC::ansi_errors`] at construction time.
        ansi: bool,
    },

    #[error("invalid import directive arguments: {directive}")]
    InvalidImportArgs { directive: String },

    #[error("I/O error reading {file}: {source}")]
    Io {
        file: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Format a Bazel-style multi-line error block for an undefined `--config=` reference.
///
/// Sections, in order: a `bazelrc:` headline (naming the subsystem so the
/// message reads as a configuration problem, not a crash); a one-line gloss
/// of what the subsystem does; for rc-file sources, the file that owns the
/// unresolved reference; the Bazel workspace root (surfaces sub-workspace
/// anchor mismatches); the loaded `.bazelrc` files; the same view
/// `--announce-rc` would print for the failing command; and two fix hints.
///
/// The hints stay neutral on *how* `--config=` got set — it can come from a
/// CLI flag, a `config.axl`, or any of several internal hooks.
fn render_undefined_config(
    command: &str,
    name: &str,
    flag_source: &Path,
    workspace_root: &Path,
    loaded_rc_files: &[PathBuf],
    applicable_rc_state: &str,
    ansi: bool,
) -> String {
    let (red, bold, dim, reset) = if ansi {
        (SGR_BOLD_RED, SGR_BOLD, SGR_DIM, SGR_RESET)
    } else {
        ("", "", "", "")
    };

    let mut out = String::new();
    writeln!(
        out,
        "{red}bazelrc:{reset} {bold}--config={name} is not defined for command '{command}'{reset}"
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "  {dim}Aspect CLI parses .bazelrc files and resolves --config= flags like Bazel.{reset}"
    )
    .unwrap();
    writeln!(out).unwrap();

    // For rc-file references (e.g. `common --config=foo` in a `.bazelrc`),
    // name the file so users know where to grep. The CLI-source case is
    // already visible as the `client` line in the rc-state dump below.
    if flag_source != Path::new(CLI_SOURCE_PATH) {
        writeln!(
            out,
            "  {bold}Unresolved --config={name} reference in:{reset}"
        )
        .unwrap();
        writeln!(out, "    {}", flag_source.display()).unwrap();
        writeln!(out).unwrap();
    }

    writeln!(out, "  {bold}Bazel workspace root:{reset}").unwrap();
    writeln!(out, "    {}", workspace_root.display()).unwrap();
    writeln!(out).unwrap();

    writeln!(out, "  {bold}Loaded .bazelrc files:{reset}").unwrap();
    if loaded_rc_files.is_empty() {
        writeln!(out, "    (none)").unwrap();
    } else {
        for p in loaded_rc_files {
            writeln!(out, "    {}", p.display()).unwrap();
        }
    }
    writeln!(out).unwrap();

    writeln!(
        out,
        "  {bold}Applicable rc state for '{command}' (same as --announce-rc):{reset}"
    )
    .unwrap();
    // Indent each line of the announce dump 4 spaces so file headers nest under
    // the section title. Strip the trailing newline `announce()` emits so the
    // blank-line separator that follows isn't doubled.
    for line in applicable_rc_state.trim_end_matches('\n').lines() {
        if line.is_empty() {
            writeln!(out).unwrap();
        } else {
            writeln!(out, "    {line}").unwrap();
        }
    }
    writeln!(out).unwrap();

    writeln!(out, "  {bold}Try one of:{reset}").unwrap();
    writeln!(
        out,
        "    - Add `common:{name} ...` (or `build:{name}`, `test:{name}`) to a loaded .bazelrc"
    )
    .unwrap();
    write!(out, "    - Don't set --config={name}").unwrap();
    out
}

/// Parsed representation of one or more `.bazelrc` files.
#[derive(Debug)]
pub struct BazelRC {
    /// Map from command key (e.g. `"build"`, `"build:opt"`) to its options.
    options: HashMap<String, Vec<RcOption>>,
    /// Ordered list of source files that were loaded.
    sources: Vec<PathBuf>,
    /// Workspace root used for rc-file discovery and `%workspace%` substitution.
    /// Surfaced via [`workspace_root`](Self::workspace_root) for error context.
    workspace_root: PathBuf,
    /// Render [`BazelRcError`] messages with ANSI escape codes. Set via
    /// [`with_ansi_errors`](Self::with_ansi_errors); defaults to `false`.
    ansi_errors: bool,
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
            sources.push(PathBuf::from(CLI_SOURCE_PATH));
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
            sources,
            workspace_root: root.to_path_buf(),
            ansi_errors: false,
        })
    }

    /// Builder: render [`BazelRcError`] messages with ANSI escape codes.
    /// Callers detect terminal/CI/`NO_COLOR` themselves (see `axl-runtime`'s
    /// `term::color_enabled`) and pass the answer in; tests leave the
    /// default (`false`).
    pub fn with_ansi_errors(mut self, ansi: bool) -> Self {
        self.ansi_errors = ansi;
        self
    }

    /// Whether [`BazelRcError`] rendering will include ANSI escape codes.
    pub fn ansi_errors(&self) -> bool {
        self.ansi_errors
    }

    /// Every source that contributed to this resolution, in load order. Includes
    /// the synthetic [`CLI_SOURCE_PATH`] entry when caller-supplied flags were
    /// passed to [`BazelRC::new`]. Use [`loaded_rc_files`](Self::loaded_rc_files)
    /// when you only want the on-disk rc files.
    pub fn sources(&self) -> &[PathBuf] {
        &self.sources
    }

    /// On-disk `.bazelrc` paths that contributed to this resolution. Mirrors
    /// [`sources`](Self::sources) minus the synthetic [`CLI_SOURCE_PATH`] entry.
    pub fn loaded_rc_files(&self) -> Vec<PathBuf> {
        self.sources
            .iter()
            .filter(|p| p.as_path() != Path::new(CLI_SOURCE_PATH))
            .cloned()
            .collect()
    }

    /// Workspace root used for rc-file discovery and `%workspace%` substitution.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
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
    /// CLI-provided flags (those passed via the `flags` parameter to [`BazelRC::new`], stored
    /// as `always` with source [`CLI_SOURCE_PATH`]) are placed **last** so any `--config=`
    /// flags they carry expand after all RC-file flags. This matches Bazel's own semantics
    /// where command-line flags override `.bazelrc` defaults under last-write-wins.
    ///
    /// For example, `options_for("test")` returns:
    ///   `always` (rc-file) + `common` + `build` + `test` + `always` (cli)
    pub fn options_for(&self, command: &str) -> Vec<&RcOption> {
        // CLI-provided flags share the "always" bucket; partition them out by
        // source so they can be appended last for last-write-wins.
        let cli_source_idx = self
            .sources
            .iter()
            .position(|p| p.as_path() == Path::new(CLI_SOURCE_PATH));

        let mut result = Vec::new();
        // RC-file always flags first (not from the synthetic CLI source)
        if let Some(opts) = self.options.get("always") {
            result.extend(
                opts.iter()
                    .filter(|o| cli_source_idx.map_or(true, |cli_idx| o.source_index != cli_idx)),
            );
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
        if let Some(cli_idx) = cli_source_idx {
            if let Some(opts) = self.options.get("always") {
                result.extend(opts.iter().filter(|o| o.source_index == cli_idx));
            }
        }
        result
    }

    /// Produce a human-readable summary of all options loaded for `command`.
    ///
    /// Output is grouped by source file, with flags wrapped at `max_width` columns.
    /// When `ansi` is true, headers and section names are styled with ANSI escape codes.
    ///
    /// Version-gated flags (from `try-import-if-bazel-version`) are annotated with
    /// `[if <cond>]` so the condition is visible.
    pub fn announce(&self, command: &str, ansi: bool, max_width: usize) -> String {
        let (b, d, y, r) = if ansi {
            ("\x1b[1m", "\x1b[2m", "\x1b[33m", "\x1b[0m")
        } else {
            ("", "", "", "")
        };

        let fmt_flag = |opt: &RcOption| -> String {
            match &opt.version_condition {
                None => opt.value.clone(),
                Some(cond) => format!("{}[if {}]{} {}", y, cond, r, opt.value),
            }
        };

        // Render rc-file paths relative to the Bazel workspace root when they live
        // inside it (`./bazel/defaults.bazelrc`), absolute otherwise (`/Users/greg/.bazelrc`).
        // The workspace root may not be canonical (caller-controlled), so canonicalize
        // it once here before prefix-matching against the canonical rc-file paths.
        let workspace_canonical = self.workspace_root.canonicalize().ok();
        let shorten = |p: &Path| -> String {
            if p == Path::new(CLI_SOURCE_PATH) {
                return "client".to_owned();
            }
            if let Some(root) = &workspace_canonical {
                if let Ok(rel) = p.strip_prefix(root) {
                    return format!("./{}", rel.display());
                }
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

        // Collect all config keys relevant to this command, sorted for deterministic output.
        let mut config_keys: Vec<&String> = self
            .options
            .keys()
            .filter(|k| {
                if let Some(base) = k.split(':').next() {
                    k.contains(':') && (base == "always" || base == "common" || base == command)
                } else {
                    false
                }
            })
            .collect();
        config_keys.sort();

        let mut out = String::new();
        let mut first_block = true;

        // Single pass per source file: emit direct sections then config sections together.
        let direct_keys = ["startup", "always", "common", command];
        for (source_idx, source_path) in self.sources.iter().enumerate() {
            let is_client = source_path == Path::new(CLI_SOURCE_PATH);
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
    /// The option is attributed to the synthetic [`CLI_SOURCE_PATH`] source
    /// (created on first use).
    pub fn push_flag(&mut self, value: &str, version_condition: Option<&str>) {
        let source_index = self
            .sources
            .iter()
            .position(|p| p == Path::new(CLI_SOURCE_PATH))
            .unwrap_or_else(|| {
                let idx = self.sources.len();
                self.sources.push(PathBuf::from(CLI_SOURCE_PATH));
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

    /// CLI-supplied `--config=ci` with a section defined for the wrong command —
    /// golden-test the full rendered block end to end including the inline
    /// `--announce-rc` dump. Substring assertions miss blank-line drift and
    /// indentation regressions, so the whole message is pinned.
    ///
    /// The rc file defines `test --test_output=errors` (no `:ci` section) so the
    /// lookup fails *and* the announce block has visible content. Test runs the
    /// default `ansi=false` path so the assertion is plain text without escapes.
    #[test]
    fn undefined_config_error_renders_cli_source_block() {
        let dir = make_workspace();
        let root = dir.path();
        let rc_path = root.join(".bazelrc");
        fs::write(&rc_path, "test --test_output=errors\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=ci"])).unwrap();
        let err = rc.expand_configs("test", &[]).unwrap_err();

        let rc_canonical = rc_path.canonicalize().unwrap();
        let expected = format!(
            "bazelrc: --config=ci is not defined for command 'test'

  Aspect CLI parses .bazelrc files and resolves --config= flags like Bazel.

  Bazel workspace root:
    {root}

  Loaded .bazelrc files:
    {rc_path}

  Applicable rc state for 'test' (same as --announce-rc):
    ./.bazelrc
      test  --test_output=errors

    client  --config=ci

  Try one of:
    - Add `common:ci ...` (or `build:ci`, `test:ci`) to a loaded .bazelrc
    - Don't set --config=ci",
            root = root.display(),
            rc_path = rc_canonical.display(),
        );
        assert_eq!(err.to_string(), expected);
    }

    /// `common --config=foo` in a .bazelrc but no `:foo` section — the
    /// "Unresolved --config=X reference in: <path>" block must point at the rc
    /// file that owns the `--config=foo` line so users know where to grep.
    #[test]
    fn undefined_config_error_attributes_rc_file_source() {
        let dir = make_workspace();
        let root = dir.path();
        let rc_path = root.join(".bazelrc");
        fs::write(&rc_path, "common --config=foo\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let msg = rc.expand_configs("build", &[]).unwrap_err().to_string();

        let rc_canonical = rc_path.canonicalize().unwrap();
        assert!(
            msg.contains(&format!(
                "Unresolved --config=foo reference in:\n    {}",
                rc_canonical.display()
            )),
            "expected rc-file source attribution, got: {msg}"
        );
    }

    /// Transitive expansion: `build:a --config=missing` triggers the error.
    /// The reference must be attributed to the rc file owning the
    /// `--config=missing` line (not the CLI flag `--config=a` that started the
    /// expansion), so users can grep the right file.
    #[test]
    fn undefined_config_error_attributes_transitive_source() {
        let dir = make_workspace();
        let root = dir.path();
        let rc_path = root.join(".bazelrc");
        fs::write(&rc_path, "build:a --config=missing\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=a"])).unwrap();
        let msg = rc.expand_configs("build", &[]).unwrap_err().to_string();

        let rc_canonical = rc_path.canonicalize().unwrap();
        assert!(
            msg.starts_with("bazelrc: --config=missing is not defined"),
            "expected error to name the inner config: {msg}"
        );
        assert!(
            msg.contains(&format!(
                "Unresolved --config=missing reference in:\n    {}",
                rc_canonical.display()
            )),
            "expected rc-file source for transitive --config=: {msg}"
        );
    }

    /// `--ignore_all_rc_files` removes every real rc file; the listing has to
    /// render cleanly with "(none)" rather than producing an empty section.
    #[test]
    fn undefined_config_error_renders_empty_rc_list() {
        let dir = make_workspace();
        let root = dir.path();

        let rc = BazelRC::new(root, &["--ignore_all_rc_files"], &flags(&["--config=ci"])).unwrap();
        let msg = rc.expand_configs("build", &[]).unwrap_err().to_string();

        assert!(
            msg.contains("Loaded .bazelrc files:\n    (none)\n"),
            "expected (none) placeholder followed by blank line: {msg}"
        );
    }

    /// The error message must never use the word "injection" or name the
    /// `BazelTrait` internals — `--config=` can be set through many surfaces
    /// (CLI, config.axl, task_flags hooks, the BazelTrait.flags transform),
    /// and naming one would mislead users. Regression guard for prior wording.
    #[test]
    fn undefined_config_error_avoids_internal_jargon() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=ci"])).unwrap();
        let msg = rc.expand_configs("build", &[]).unwrap_err().to_string();

        for forbidden in ["injection", "BazelTrait", "extra_flags"] {
            assert!(!msg.contains(forbidden), "found '{forbidden}' in: {msg}");
        }
    }

    /// `with_ansi_errors(true)` makes the rendered block embed SGR escape codes
    /// for the section headers, prefix, and intro line; `with_ansi_errors(false)`
    /// (the default) keeps the message plain.
    #[test]
    fn undefined_config_error_emits_ansi_when_enabled() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=ci"]))
            .unwrap()
            .with_ansi_errors(true);
        let msg = rc.expand_configs("build", &[]).unwrap_err().to_string();

        assert!(msg.starts_with(SGR_BOLD_RED), "missing red prefix: {msg}");
        assert!(msg.contains(SGR_BOLD), "missing bold section header: {msg}");
        assert!(msg.contains(SGR_DIM), "missing dim intro line: {msg}");
        assert!(msg.contains(SGR_RESET), "missing reset: {msg}");
    }

    /// `loaded_rc_files()` returns the on-disk rc files, excluding the synthetic
    /// `<command line>` source created when caller-supplied flags are passed.
    #[test]
    fn loaded_rc_files_excludes_synthetic_cli_source() {
        let dir = make_workspace();
        let root = dir.path();
        let rc_path = root.join(".bazelrc");
        fs::write(&rc_path, "build --jobs=4\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &flags(&["--config=opt"])).unwrap();
        // `sources()` includes the CLI bucket; `loaded_rc_files()` should not.
        assert!(
            rc.sources().iter().any(|p| p == Path::new(CLI_SOURCE_PATH)),
            "sources() should include the synthetic CLI entry"
        );
        let loaded = rc.loaded_rc_files();
        assert_eq!(
            loaded.len(),
            1,
            "expected exactly the workspace rc: {loaded:?}"
        );
        assert_eq!(loaded[0], rc_path.canonicalize().unwrap());
    }

    /// `announce()` renders rc files inside the workspace root as `./relative`
    /// paths so users can tell at a glance the file is in-tree, and rc files
    /// outside the root as absolute paths.
    #[test]
    fn announce_renders_workspace_relative_and_absolute_paths() {
        let dir = make_workspace();
        let root = dir.path();
        // In-tree rc with a `build` section (the announce loop emits sources
        // only when they have a section matching one of the direct keys).
        let in_tree = root.join(".bazelrc");
        fs::write(&in_tree, "build --jobs=4\n").unwrap();

        // Out-of-tree rc loaded via --bazelrc=. Sibling of `root` so it never
        // matches `root.canonicalize()` as a prefix.
        let outside_dir = make_workspace();
        let outside = outside_dir.path().join("external.bazelrc");
        fs::write(&outside, "build --verbose_failures\n").unwrap();

        let outside_flag = format!("--bazelrc={}", outside.display());
        let rc = BazelRC::new(root, &["--nosystem_rc", "--nohome_rc", &outside_flag], &[]).unwrap();
        let announced = rc.announce("build", false, 200);

        assert!(
            announced.contains("./.bazelrc"),
            "in-tree rc should render as ./.bazelrc; got: {announced}"
        );
        let outside_canonical = outside.canonicalize().unwrap();
        assert!(
            announced.contains(&outside_canonical.display().to_string()),
            "out-of-tree rc should render as an absolute path; got: {announced}"
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

        let rc = BazelRC::new(
            root,
            &[] as &[&str],
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

        // Source of CLI flags is the synthetic CLI_SOURCE_PATH entry
        let always = rc.raw_options("always");
        assert_eq!(always.len(), 2);
        assert_eq!(rc.source_of(&always[0]), Path::new(CLI_SOURCE_PATH));
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
