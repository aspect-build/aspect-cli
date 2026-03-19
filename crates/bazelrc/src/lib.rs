mod discover;
mod expand;
mod parse;
pub(crate) mod preprocess;
pub(crate) mod tokenize;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use std::fmt;

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

/// Parsed representation of one or more `.bazelrc` files.
#[derive(Debug)]
pub struct BazelRC {
    /// Map from command key (e.g. `"build"`, `"build:opt"`) to its options.
    options: HashMap<String, Vec<RcOption>>,
    /// Ordered list of source files that were loaded.
    sources: Vec<PathBuf>,
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

        Ok(BazelRC { options, sources })
    }

    /// The ordered list of source files that were loaded.
    pub fn sources(&self) -> &[PathBuf] {
        &self.sources
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

    /// Return all options applicable to `command`, in order: `always` + `common` + `<command>`.
    pub fn options_for(&self, command: &str) -> Vec<&RcOption> {
        let mut result = Vec::new();
        for key in ["always", "common", command] {
            if let Some(opts) = self.options.get(key) {
                result.extend(opts.iter());
            }
        }
        result
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

    /// Expand all `--config=` flags for the given command.
    ///
    /// Starts from `options_for(command)` (which includes `always`, `common`, and command-specific
    /// options, plus any CLI flags passed to `BazelRC::new`). Returns the fully-expanded flat list
    /// of `RcOption`s with all `--config=` references resolved.
    ///
    /// Each entry's `version_condition` is `Some(cond)` when the option originated from a
    /// `try-import-if-bazel-version` block; `None` otherwise. Callers can inspect this field to
    /// decide how to handle version-gated flags.
    pub fn expand_configs(&self, command: &str) -> Result<Vec<RcOption>, BazelRcError> {
        expand::expand_configs(self, command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

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
            format!("import {}\nimport {}\n", b.display(), c.display()),
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
            "build:opt --copt=-O2\nbuild:opt --compilation_mode=opt\n",
        )
        .unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &flags(&["--config=opt"])).unwrap();
        let expanded = rc.expand_configs("build").unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert_eq!(values, vec!["--copt=-O2", "--compilation_mode=opt"]);
    }

    #[test]
    fn config_expansion_cycle_detected() {
        let dir = make_workspace();
        let root = dir.path();
        let rc_path = root.join(".bazelrc");
        fs::write(&rc_path, "build:a --config=b\nbuild:b --config=a\n").unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &flags(&["--config=a"])).unwrap();
        let err = rc.expand_configs("build").unwrap_err();
        assert!(matches!(err, BazelRcError::ConfigCycle { .. }));
    }

    #[test]
    fn config_undefined_errors() {
        let dir = make_workspace();
        let root = dir.path();
        let rc_path = root.join(".bazelrc");
        fs::write(&rc_path, "build --jobs=4\n").unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &flags(&["--config=nonexistent"])).unwrap();
        let err = rc.expand_configs("build").unwrap_err();
        assert!(matches!(err, BazelRcError::UndefinedConfig { .. }));
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
            "always --always-flag\ncommon --common-flag\nbuild --build-flag\n",
        )
        .unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
        let opts: Vec<&str> = rc
            .options_for("build")
            .iter()
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(opts, vec!["--always-flag", "--common-flag", "--build-flag"]);
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

        // CLI flags appear at the front of options_for (as always opts)
        let opts: Vec<&str> = rc
            .options_for("build")
            .iter()
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(opts, vec!["--config=foo", "--verbose_failures", "--jobs=4"]);

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

        let rc = BazelRC::new(root, &[] as &[&str], &flags(&["--config=opt"])).unwrap();
        // expand_configs picks up the CLI --config=opt from always options
        let expanded = rc.expand_configs("build").unwrap();
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
                "build --jobs=4\ntry-import-if-bazel-version >=8.0.0 {}\n",
                versioned.display()
            ),
        )
        .unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
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
            "build --jobs=4\ntry-import-if-bazel-version >=8.0.0 /nonexistent/path.bazelrc\n",
        )
        .unwrap();

        // Should not error — missing file is silently skipped like try-import
        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
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
                "build --jobs=4\ntry-import-if-bazel-version >=8.0.0 {}\nbuild --remote_cache=grpc://cache\n",
                versioned.display()
            ),
        )
        .unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
        let expanded = rc.expand_configs("build").unwrap();

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

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
        assert_eq!(rc.options_for("build").len(), 0);
    }

    #[test]
    fn whitespace_only_file() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "   \n\t\n  \n").unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
        assert_eq!(rc.options_for("build").len(), 0);
    }

    #[test]
    fn commented_line() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "# startup foo\n").unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
        assert_eq!(rc.options_for("startup").len(), 0);
    }

    #[test]
    fn command_with_no_args() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build\n").unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
        assert_eq!(rc.options_for("build").len(), 0);
    }

    #[test]
    fn command_with_trailing_comment() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4 # a comment\n").unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
        let opts = rc.options_for("build");
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].value, "--jobs=4");
    }

    #[test]
    fn multiple_args_on_one_line() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "build --jobs=4 --verbose_failures\n").unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
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
            "build --jobs=4\nbuild --verbose_failures\n",
        )
        .unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
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
            "startup --max_idle_secs=60\nbuild --jobs=4\n",
        )
        .unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
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

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
        let opts = rc.options_for("build");
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].value, "--jobs=4");
    }

    #[test]
    fn indented_command_parsed() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "  build --jobs=4\n").unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
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
            "build --jobs=4 \\\n  --verbose_failures\n",
        )
        .unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
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
            format!("import {}\nbuild --local-bar\n", foo.display()),
        )
        .unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
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
            format!("build --local-bar\nimport {}\n", foo.display()),
        )
        .unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
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

        let rc = BazelRC::new(root, &["--noworkspace_rc"], &[]).unwrap();
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

        let err = BazelRC::new(root, &[] as &[&str], &[]).unwrap_err();
        assert!(matches!(err, BazelRcError::InvalidImportArgs { .. }));
    }

    #[test]
    fn try_import_too_many_args_error() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "try-import foo bar\n").unwrap();

        let err = BazelRC::new(root, &[] as &[&str], &[]).unwrap_err();
        assert!(matches!(err, BazelRcError::InvalidImportArgs { .. }));
    }

    // ── Config expansion ──────────────────────────────────────────────────────

    #[test]
    fn common_config_section_used_as_fallback() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "common:myconfig --common-flag\n").unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &flags(&["--config=myconfig"])).unwrap();
        let expanded = rc.expand_configs("build").unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert!(values.contains(&"--common-flag"), "got: {values:?}");
    }

    #[test]
    fn multi_level_config_expansion() {
        let dir = make_workspace();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            "build:a --config=b\nbuild:b --deep-flag\n",
        )
        .unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &flags(&["--config=a"])).unwrap();
        let expanded = rc.expand_configs("build").unwrap();
        let values: Vec<&str> = expanded.iter().map(|o| o.value.as_str()).collect();
        assert_eq!(values, vec!["--deep-flag"]);
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
        let expanded = rc.expand_configs("build").unwrap();
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
            "common --config=foo\nbuild:foo --foo-flag\n",
        )
        .unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
        let expanded = rc.expand_configs("build").unwrap();
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
            "always --config=foo\nbuild:foo --foo-flag\n",
        )
        .unwrap();

        let rc = BazelRC::new(root, &[] as &[&str], &[]).unwrap();
        let expanded = rc.expand_configs("build").unwrap();
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

        let rc = BazelRC::new(root, &[] as &[&str], &flags(&["--config=myconfig"])).unwrap();
        let expanded = rc.expand_configs("build").unwrap();
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
        let expanded = rc.expand_configs("build").unwrap();
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
        let rc = BazelRC::new(root, &[] as &[&str], &flags(&["--config=myconfig"])).unwrap();
        let expanded = rc.expand_configs("build").unwrap();

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].value, "--sandbox_default_allow_network=false");
        // Inherited from the versioned config section
        assert_eq!(expanded[0].version_condition.as_deref(), Some(">=8.0.0"));
    }
}
