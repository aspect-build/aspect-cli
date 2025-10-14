use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;

use starlark::{
    environment::GlobalsBuilder, starlark_module,
    values::starlark_value_as_type::StarlarkValueAsType,
};

mod env;
mod fs;
mod io;
mod process;
mod stream;
mod stream_iter;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std>")]
pub struct Std {}

starlark_simple_value!(Std);

#[starlark_value(type = "std")]
impl<'v> values::StarlarkValue<'v> for Std {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(std_methods)
    }
}

#[starlark_module]
pub(crate) fn std_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn env<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<env::Env> {
        Ok(env::Env::new())
    }

    #[starlark(attribute)]
    fn io<'v>(this: values::Value<'v>) -> starlark::Result<io::Stdio> {
        Ok(io::Stdio::new())
    }

    #[starlark(attribute)]
    fn fs<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<fs::Filesystem> {
        Ok(fs::Filesystem::new())
    }

    #[starlark(attribute)]
    fn process<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<process::Process> {
        Ok(process::Process::new())
    }
}

#[starlark_module]
fn toplevels(builder: &mut GlobalsBuilder) {
    const std: StarlarkValueAsType<Std> = StarlarkValueAsType::new();

    const fs: StarlarkValueAsType<fs::Filesystem> = StarlarkValueAsType::new();
    const env: StarlarkValueAsType<env::Env> = StarlarkValueAsType::new();
}

#[starlark_module]
fn process_toplevels(builder: &mut GlobalsBuilder) {
    const process: StarlarkValueAsType<process::Process> = StarlarkValueAsType::new();
    const child: StarlarkValueAsType<process::Child> = StarlarkValueAsType::new();
    const command: StarlarkValueAsType<process::Command> = StarlarkValueAsType::new();
    const exit_status: StarlarkValueAsType<process::ExitStatus> = StarlarkValueAsType::new();
    const output: StarlarkValueAsType<process::Output> = StarlarkValueAsType::new();
}

#[starlark_module]
fn io_toplevels(builder: &mut GlobalsBuilder) {
    const Stdio: StarlarkValueAsType<io::Stdio> = StarlarkValueAsType::new();
    const WritebleStream: StarlarkValueAsType<stream::Writable> = StarlarkValueAsType::new();
    const ReadableStream: StarlarkValueAsType<stream::Readable> = StarlarkValueAsType::new();
}

pub fn register_toplevels(builder: &mut GlobalsBuilder) {
    toplevels(builder);
    builder.namespace("process", process_toplevels);
    builder.namespace("io", io_toplevels);
}
