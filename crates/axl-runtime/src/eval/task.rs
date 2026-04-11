use anyhow::anyhow;
use starlark::environment::FrozenModule;
use starlark::values::{OwnedFrozenValue, ValueLike};

use crate::engine::task::FrozenTask;

use super::error::EvalError;

/// Trait for introspection operations on frozen modules.
pub trait FrozenTaskModuleLike {
    fn tasks(&self) -> Vec<String>;
    fn has_task(&self, symbol: &str) -> bool;
    fn has_name(&self, symbol: &str) -> bool;
    /// Retrieves a task definition from the frozen module by symbol name.
    fn get_task(&self, symbol: &str) -> Result<OwnedFrozenValue, EvalError>;
}

impl FrozenTaskModuleLike for FrozenModule {
    fn get_task(&self, symbol: &str) -> Result<OwnedFrozenValue, EvalError> {
        let def = self
            .get(symbol)
            .map_err(|e| EvalError::UnknownError(anyhow!(e)))?;
        // Verify it's actually a task
        let value = def.value();
        if value.downcast_ref::<FrozenTask>().is_none() {
            return Err(EvalError::UnknownError(anyhow!("expected type of Task")));
        }
        Ok(def)
    }

    fn tasks(&self) -> Vec<String> {
        self.names()
            .filter(|symbol| self.has_task(symbol.as_str()))
            .map(|sym| sym.as_str().to_string())
            .collect()
    }

    fn has_task(&self, symbol: &str) -> bool {
        if let Ok(val) = self.get(symbol) {
            if val.value().downcast_ref::<FrozenTask>().is_some() {
                return true;
            }
        }
        false
    }

    fn has_name(&self, symbol: &str) -> bool {
        self.get(symbol).is_ok()
    }
}
