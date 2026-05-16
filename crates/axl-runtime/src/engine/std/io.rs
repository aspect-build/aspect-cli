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

use super::stream;

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
    pub fn new() -> Self {
        Self {
            stdout: stream::Writable::from(std::io::stdout()),
            stderr: stream::Writable::from(std::io::stderr()),
            stdin: stream::Readable::from(std::io::stdin()),
        }
    }
}

#[starlark_value(type = "std.io.Stdio")]
impl<'v> values::StarlarkValue<'v> for Stdio {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(stdio_methods)
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
}
