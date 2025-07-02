use std::fmt::Debug;

use allocative::Allocative;
use derive_more::Display;
use dupe::Dupe;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use super::stream;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<stdio>")]
pub struct Stdio {
    stdout: stream::Writable,
    stderr: stream::Writable,
    stdin: stream::Readable,
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

#[starlark_value(type = "stdio")]
impl<'v> values::StarlarkValue<'v> for Stdio {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(stdio_methods)
    }
}

starlark_simple_value!(Stdio);

#[starlark_module]
pub(crate) fn stdio_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn stdout<'v>(this: values::Value) -> anyhow::Result<stream::Writable> {
        let this = this.downcast_ref_err::<Stdio>()?;
        Ok(this.stdout.dupe())
    }
    #[starlark(attribute)]
    fn stderr<'v>(this: values::Value) -> anyhow::Result<stream::Writable> {
        let this = this.downcast_ref_err::<Stdio>()?;
        Ok(this.stderr.dupe())
    }
    #[starlark(attribute)]
    fn stdin<'v>(this: values::Value) -> anyhow::Result<stream::Readable> {
        let this = this.downcast_ref_err::<Stdio>()?;
        Ok(this.stdin.dupe())
    }
}
