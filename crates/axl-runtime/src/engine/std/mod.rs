use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::starlark_value;

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
#[display("<std.Std>")]
pub struct Std {}

starlark_simple_value!(Std);

#[starlark_value(type = "std.Std")]
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
fn register_types(globals: &mut GlobalsBuilder) {
    const Env: StarlarkValueAsType<env::Env> = StarlarkValueAsType::new();
    const FileSystem: StarlarkValueAsType<fs::Filesystem> = StarlarkValueAsType::new();
    const Std: StarlarkValueAsType<Std> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_process_types(globals: &mut GlobalsBuilder) {
    const Child: StarlarkValueAsType<process::Child> = StarlarkValueAsType::new();
    const Command: StarlarkValueAsType<process::Command> = StarlarkValueAsType::new();
    const ExitStatus: StarlarkValueAsType<process::ExitStatus> = StarlarkValueAsType::new();
    const Output: StarlarkValueAsType<process::Output> = StarlarkValueAsType::new();
    const Process: StarlarkValueAsType<process::Process> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_io_types(globals: &mut GlobalsBuilder) {
    const Stdio: StarlarkValueAsType<io::Stdio> = StarlarkValueAsType::new();
    const Readable: StarlarkValueAsType<stream::Readable> = StarlarkValueAsType::new();
    const Writable: StarlarkValueAsType<stream::Writable> = StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    register_types(globals);

    globals.namespace("process", register_process_types);
    globals.namespace("io", register_io_types);
}
