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
        let opts = this
            .inner
            .expand_configs(&command)
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
        let opts = this
            .inner
            .expand_configs(&command)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let mut startup_flags: Vec<Value<'v>> = Vec::new();
        let mut flags: Vec<Value<'v>> = Vec::new();

        for opt in this.inner.raw_options("startup") {
            let v = match &opt.version_condition {
                None => eval.heap().alloc(opt.value.as_str()),
                Some(cond) => eval
                    .heap()
                    .alloc((opt.value.as_str() as &str, cond.as_str() as &str)),
            };
            startup_flags.push(v);
        }

        for opt in &opts {
            let base = opt.command.split(':').next().unwrap_or(&opt.command);

            if base == "common" {
                let override_str = format!("--default_override=0:common={}", opt.value);
                let v = match &opt.version_condition {
                    None => eval.heap().alloc(override_str.as_str()),
                    Some(cond) => eval
                        .heap()
                        .alloc((override_str.as_str() as &str, cond.as_str() as &str)),
                };
                flags.push(v);
            } else {
                let v = match &opt.version_condition {
                    None => eval.heap().alloc(opt.value.as_str()),
                    Some(cond) => eval
                        .heap()
                        .alloc((opt.value.as_str() as &str, cond.as_str() as &str)),
                };
                flags.push(v);
            }
        }

        Ok((startup_flags, flags))
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
