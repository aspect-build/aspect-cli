use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use starlark::{
    environment::GlobalsBuilder, starlark_module,
    values::starlark_value_as_type::StarlarkValueAsType,
};

use crate::engine::store::TestEnvMap;

mod env;
mod fs;
pub mod io;
mod net;
mod process;
pub mod stream;

#[derive(Clone, Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.Std>")]
pub struct Std {
    /// When `Some`, the `std.Env` minted by `std.env` carries this in-memory
    /// overlay (the mock route is value-carried, not ambient). Production
    /// leaves this `None`; the test runner mints `Std` carrying the test's
    /// shared overlay `Rc` so `t.std.env` and `t.ctx.std.env` observe one map.
    pub env_overlay: Option<TestEnvMap>,
}

impl Std {
    /// Production constructor: no env overlay (real process env).
    pub fn new() -> Self {
        Self { env_overlay: None }
    }

    /// Test constructor: the `std.env` it hands out reads/writes `overlay`.
    pub fn with_env_overlay(overlay: TestEnvMap) -> Self {
        Self {
            env_overlay: Some(overlay),
        }
    }
}

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
    fn env<'v>(this: values::Value<'v>) -> anyhow::Result<env::Env> {
        let std = this
            .downcast_ref::<Std>()
            .ok_or_else(|| anyhow::anyhow!("std.env accessed on a non-std.Std value"))?;
        Ok(match &std.env_overlay {
            Some(overlay) => env::Env::with_overlay(overlay.clone()),
            None => env::Env::new(),
        })
    }

    #[starlark(attribute)]
    fn io<'v>(this: values::Value<'v>) -> anyhow::Result<io::Stdio> {
        Ok(io::Stdio::new())
    }

    #[starlark(attribute)]
    fn fs<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<fs::Filesystem> {
        Ok(fs::Filesystem::new())
    }

    #[starlark(attribute)]
    fn process<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<process::Process> {
        Ok(process::Process::new())
    }

    #[starlark(attribute)]
    fn net<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<net::Net> {
        Ok(net::Net::new())
    }
}

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const Env: StarlarkValueAsType<env::Env> = StarlarkValueAsType::new();
    const FileSystem: StarlarkValueAsType<fs::Filesystem> = StarlarkValueAsType::new();
    const Net: StarlarkValueAsType<net::Net> = StarlarkValueAsType::new();
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
