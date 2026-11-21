use std::fmt::Debug;

use allocative::Allocative;
use derive_more::Display;
use dupe::Dupe;
use starlark::StarlarkResultExt;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::UnpackValue;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use std::sync::Arc;

use super::stream;
use crate::color::StreamColor;
use crate::engine::cancellation::Signals;

#[derive(Debug, Display, Clone, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.io.Stdio>")]
pub struct Stdio {
    pub stdout: stream::Writable,
    pub stderr: stream::Writable,
    pub stdin: stream::Readable,
}

impl<'v> UnpackValue<'v> for Stdio {
    type Error = anyhow::Error;

    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        let v = value.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(Some(v.clone()))
    }
}

impl Stdio {
    /// The process's streams, stdout and stderr reporting `color` as their
    /// `color`. A blocking read of stdin is a safe point: it ends with the
    /// task's exit once the root token is cancelled, so a cancelled run is not
    /// stuck behind a prompt.
    pub fn new(signals: Arc<Signals>, color: StreamColor) -> Self {
        let stdin = stream::Readable::from(std::io::stdin());
        stdin.set_stdin_interrupt(Arc::new(move || {
            signals
                .should_unwind()
                .then(|| std::io::Error::other(signals.exit_for()))
        }));
        Self {
            stdout: stream::Writable::stdout(std::io::stdout(), color.stdout),
            stderr: stream::Writable::stderr(std::io::stderr(), color.stderr),
            stdin,
        }
    }
}

#[starlark_value(type = "std.io.Stdio")]
impl<'v> values::StarlarkValue<'v> for Stdio {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("stdio_methods", stdio_methods);
        Some(RES.methods())
    }
}

starlark_simple_value!(Stdio);

#[starlark_module]
pub(crate) fn stdio_methods(registry: &mut MethodsBuilder) {
    /// Returns a writable stream for the standard output of the current process.
    #[starlark(attribute)]
    fn stdout<'v>(this: values::Value) -> anyhow::Result<stream::Writable> {
        let this = this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(this.stdout.dupe())
    }

    /// Returns a writable stream for the standard error of the current process.
    #[starlark(attribute)]
    fn stderr<'v>(this: values::Value) -> anyhow::Result<stream::Writable> {
        let this = this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(this.stderr.dupe())
    }

    /// Returns a readable stream for the standard input of the current process.
    #[starlark(attribute)]
    fn stdin<'v>(this: values::Value) -> anyhow::Result<stream::Readable> {
        let this = this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(this.stdin.dupe())
    }

    /// Whether detail lines print: the `INFO:` asides a reader can do
    /// without. The `Console` feature decides for the running task: its preset
    /// (shown under `full` and `plain`, hidden under `compact`, `agent` and
    /// `silent`) unless its `verbose` arg overrides it. True before the console
    /// resolves. `detail()` in `lib/environment.axl` follows it; `info()`
    /// always prints.
    #[starlark(attribute)]
    fn verbose<'v>(this: values::Value) -> anyhow::Result<bool> {
        this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(crate::diag::verbose())
    }
}
