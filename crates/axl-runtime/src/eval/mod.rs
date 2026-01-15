mod api;
pub mod config;
mod error;
mod load;
mod load_path;
pub mod task;

pub use api::get_globals;
pub use error::EvalError;
pub use load::AxlLoader as Loader;
pub use load::ModuleScope;
pub(crate) use load_path::validate_module_name;

// Task execution and introspection
pub use task::execute_task;
pub use task::execute_task_with_args;
pub use task::FrozenTaskModuleLike;
