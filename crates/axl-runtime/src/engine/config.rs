// engine::config
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StarlarkValue;
use starlark::values::ValueLike;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<config>")]
pub struct Config {}

starlark_simple_value!(Config);

#[starlark_value(type = "config")]
impl<'v> StarlarkValue<'v> for Config {}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<config_builder>")]
pub struct ConfigBuilder {
    tools: Mutex<HashMap<String, PathBuf>>,
}

starlark_simple_value!(ConfigBuilder);

#[starlark_value(type = "config_builder")]
impl<'v> StarlarkValue<'_> for ConfigBuilder {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(config_context_methods)
    }
}

#[starlark_module]
pub(crate) fn config_context_methods(registry: &mut MethodsBuilder) {
    /// Standard library is the foundation of powerful AXL tasks.
    fn add_tool<'v>(
        this: values::Value<'v>,
        #[starlark()] tool_name: values::StringValue,
        tool_path: values::StringValue,
    ) -> starlark::Result<NoneType> {
        let this = this.downcast_ref::<ConfigBuilder>().unwrap();
        let mut tools = this.tools.lock().unwrap();
        if !tools.contains_key(&tool_name.to_string()) {
            tools.insert(tool_name.to_string(), PathBuf::from(tool_path.to_string()));
            return Ok(NoneType);
        } else {
            return Err(anyhow::format_err!(
                "Error: tool {} was registered twice!",
                tool_name.to_string()
            )
            .into());
        }
    }
}

#[starlark_module]
pub fn register_toplevels(_: &mut GlobalsBuilder) {
    /// Task type representing a Task.
    ///
    /// ```python
    /// def _task_impl(ctx):
    ///     pass
    ///
    /// build = task(
    ///     impl = _task_impl,
    ///     task_args = {
    ///         "target": args.string(),
    ///     }
    ///     groups = [],
    /// )
    /// ```
    const config_builder: StarlarkValueAsType<ConfigBuilder> = StarlarkValueAsType::new();

    #[starlark(as_type = ConfigBuilder)]
    fn config_builder<'v>() -> starlark::Result<ConfigBuilder> {
        Ok(ConfigBuilder {
            tools: Mutex::new(HashMap::new()),
        })
    }
}
