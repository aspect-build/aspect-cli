use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::starlark_value;

#[derive(Clone, Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<TaskInfo>")]
pub struct TaskInfo {
    pub name: String,
    pub group: Vec<String>,
    pub task_key: String,
    pub task_id: String,
}

starlark_simple_value!(TaskInfo);

#[starlark_value(type = "TaskInfo")]
impl<'v> values::StarlarkValue<'v> for TaskInfo {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(task_info_methods)
    }
}

#[starlark_module]
fn task_info_methods(registry: &mut MethodsBuilder) {
    /// The name of the task.
    #[starlark(attribute)]
    fn name(this: &TaskInfo) -> anyhow::Result<String> {
        Ok(this.name.clone())
    }

    /// The group(s) this task belongs to.
    #[starlark(attribute)]
    fn group(this: &TaskInfo) -> anyhow::Result<Vec<String>> {
        Ok(this.group.clone())
    }

    /// A short human-readable key identifying this task invocation.
    /// Set via --task-key on the CLI, or auto-generated as a friendly name (e.g. "fluffy-parakeet").
    #[starlark(attribute)]
    fn key(this: &TaskInfo) -> anyhow::Result<String> {
        Ok(this.task_key.clone())
    }

    /// A globally unique UUID v4 for this task invocation.
    /// Always auto-generated; use key for a short human-readable discriminator.
    #[starlark(attribute)]
    fn id(this: &TaskInfo) -> anyhow::Result<String> {
        Ok(this.task_id.clone())
    }
}
