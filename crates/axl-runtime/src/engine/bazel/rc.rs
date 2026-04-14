use allocative::Allocative;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Value;
use starlark::values::starlark_value;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("{inner}")]
pub struct StarlarkBazelRC {
    #[allocative(skip)]
    pub inner: bazelrc::BazelRC,
    pub skip_config_if_missing: Vec<String>,
}

starlark_simple_value!(StarlarkBazelRC);

#[starlark_value(type = "bazel.BazelRC")]
impl<'v> values::StarlarkValue<'v> for StarlarkBazelRC {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(bazelrc_methods)
    }
}

#[starlark::starlark_module]
fn bazelrc_methods(registry: &mut MethodsBuilder) {
    /// Return all options applicable to `command` without expanding `--config=` flags.
    ///
    /// Each item is either a plain `str` (unconditional) or a `(str, str)` tuple
    /// `(flag, version_condition)` for version-gated flags.
    fn options_for<'v>(
        this: &StarlarkBazelRC,
        #[starlark(require = named)] command: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        let opts = this.inner.options_for(&command);
        let mut result = Vec::with_capacity(opts.len());
        for opt in opts {
            let v = match &opt.version_condition {
                None => eval.heap().alloc(opt.value.as_str()),
                Some(cond) => eval
                    .heap()
                    .alloc((opt.value.as_str() as &str, cond.as_str() as &str)),
            };
            result.push(v);
        }
        Ok(result)
    }

    /// Expand all `--config=` flags for `command` and return the fully-resolved list.
    ///
    /// Each item is either a plain `str` (unconditional) or a `(str, str)` tuple
    /// `(flag, version_condition)` for version-gated flags. This format is directly
    /// compatible with `ctx.bazel.build(flags=...)`.
    fn expand<'v>(
        this: &StarlarkBazelRC,
        #[starlark(require = named)] command: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        let ignore: Vec<&str> = this
            .skip_config_if_missing
            .iter()
            .map(|s| s.as_str())
            .collect();
        let opts = this
            .inner
            .expand_configs(&command, &ignore)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let mut result = Vec::with_capacity(opts.len());
        for opt in &opts {
            let v = match &opt.version_condition {
                None => eval.heap().alloc(opt.value.as_str()),
                Some(cond) => eval
                    .heap()
                    .alloc((opt.value.as_str() as &str, cond.as_str() as &str)),
            };
            result.push(v);
        }
        Ok(result)
    }

    /// Expand all `--config=` flags for `command` and split results by origin section.
    ///
    /// Options from `common` sections cannot be passed directly on the Bazel CLI — they must be
    /// injected via startup flags so Bazel applies the correct silent-ignore semantics. All other
    /// options (`always`, `build`, etc.) are safe to pass as regular command flags.
    ///
    /// Returns a `(startup_flags, flags)` tuple:
    /// - `startup_flags`: `["--default_override=0:common=<value>", ...]`
    /// - `flags`: direct command flags (same `str | (str, str)` format as `expand()`)
    ///
    /// # Example
    /// ```python
    /// startup_flags, flags = rc.expand_all(command = "build")
    /// ctx.bazel.build("//...", flags = flags, startup_flags = startup_flags)
    /// ```
    fn expand_all<'v>(
        this: &StarlarkBazelRC,
        #[starlark(require = named)] command: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<(Vec<Value<'v>>, Vec<Value<'v>>)> {
        let ignore: Vec<&str> = this
            .skip_config_if_missing
            .iter()
            .map(|s| s.as_str())
            .collect();
        let opts = this
            .inner
            .expand_configs(&command, &ignore)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let mut startup_flags: Vec<Value<'v>> = Vec::new();

        for opt in this.inner.raw_options("startup") {
            let v = match &opt.version_condition {
                None => eval.heap().alloc(opt.value.as_str()),
                Some(cond) => eval
                    .heap()
                    .alloc((opt.value.as_str() as &str, cond.as_str() as &str)),
            };
            startup_flags.push(v);
        }

        // Split opts: common section → --default_override flags (prepended so user flags win),
        // everything else → direct flags.
        let (override_strings, flag_strings) = partition_expand_all(&opts);

        let mut flags: Vec<Value<'v>> = Vec::new();
        for s in override_strings.iter().chain(flag_strings.iter()) {
            flags.push(eval.heap().alloc(s.as_str()));
        }
        Ok((startup_flags, flags))
    }

    /// Return a human-readable summary of all options loaded for `command`.
    ///
    /// Options are grouped by source file and wrapped at `max_width` columns (default 120).
    /// Pass `ansi = True` to enable bold/dim ANSI styling on headers and section names.
    ///
    /// Useful for debugging which flags were picked up and from which rc file:
    ///
    /// ```python
    /// print(rc.announce(command = "build"))
    /// print(rc.announce(command = "build", ansi = True))
    /// ```
    fn announce(
        this: &StarlarkBazelRC,
        #[starlark(require = named)] command: String,
        #[starlark(require = named, default = false)] ansi: bool,
        #[starlark(require = named, default = 120)] max_width: i64,
    ) -> anyhow::Result<String> {
        Ok(this
            .inner
            .announce(&command, ansi, max_width.max(0) as usize))
    }

    /// Return the list of source file paths that were loaded.
    fn sources<'v>(
        this: &StarlarkBazelRC,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Vec<Value<'v>>> {
        let mut result = Vec::with_capacity(this.inner.sources().len());
        for p in this.inner.sources() {
            result.push(eval.heap().alloc(p.display().to_string()));
        }
        Ok(result)
    }
}

/// Split an expanded option list into `(default_override_flags, regular_flags)`.
///
/// `common` section options are converted to `--default_override=0:common=<value>` strings and
/// returned first so they appear before user-specified flags in the final `flags` output.
/// This preserves last-write-wins semantics: user flags (which come later) override the defaults.
fn partition_expand_all(opts: &[bazelrc::RcOption]) -> (Vec<String>, Vec<String>) {
    let mut default_override_flags = Vec::new();
    let mut flags = Vec::new();
    for opt in opts {
        let base = opt.command.split(':').next().unwrap_or(&opt.command);
        if base == "common" {
            default_override_flags.push(format!("--default_override=0:common={}", opt.value));
        } else {
            flags.push(opt.value.clone());
        }
    }
    (default_override_flags, flags)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use bazelrc::{BazelRC, RcOption};
    use tempfile::tempdir;

    use super::partition_expand_all;

    const ISOLATE: &[&str] = &["--nosystem_rc", "--nohome_rc"];

    fn cli_flag(value: &str) -> RcOption {
        RcOption {
            value: value.to_string(),
            command: "always".to_string(),
            ..RcOption::default()
        }
    }

    // ── expand_all ordering (Bug #2) ─────────────────────────────────────────

    #[test]
    fn default_override_flags_precede_user_flags() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join(".bazelrc"),
            "common --remote_cache=grpc://cache\nbuild --jobs=4\n",
        )
        .unwrap();

        // Simulate a user passing a CLI flag; it is stored as `always`.
        let rc = BazelRC::new(root, ISOLATE, &[cli_flag("--user-flag")]).unwrap();
        let opts = rc.expand_configs("build", &[]).unwrap();

        let (overrides, regular) = partition_expand_all(&opts);

        // --default_override flags must all appear before any regular flag so
        // that user-specified flags (coming later) take precedence.
        let all: Vec<&str> = overrides
            .iter()
            .chain(regular.iter())
            .map(String::as_str)
            .collect();

        let first_override = all
            .iter()
            .position(|s| s.starts_with("--default_override"))
            .expect("--default_override flag missing");
        let first_user = all
            .iter()
            .position(|s| *s == "--user-flag")
            .expect("--user-flag missing");

        assert!(
            first_override < first_user,
            "--default_override must come before user-specified flags; got: {all:?}"
        );
    }

    #[test]
    fn common_flags_become_default_override_entries() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".bazelrc"), "common --foo\ncommon --bar\n").unwrap();

        let rc = BazelRC::new(root, ISOLATE, &[]).unwrap();
        let opts = rc.expand_configs("build", &[]).unwrap();
        let (overrides, regular) = partition_expand_all(&opts);

        assert_eq!(
            overrides,
            vec![
                "--default_override=0:common=--foo",
                "--default_override=0:common=--bar",
            ]
        );
        assert!(regular.is_empty());
    }
}
