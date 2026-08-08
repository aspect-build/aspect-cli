use starlark::environment::FrozenModule;
use starlark::values::ValueLike;

use crate::engine::guidance::Guidance;

/// Frozen-module introspection for guidance topics, mirroring
/// [`super::task::FrozenTaskModuleLike`].
pub trait FrozenGuidanceModuleLike {
    /// Every `Guidance` bound to a top-level symbol, cloned out of the module.
    ///
    /// Cloning rather than holding heap references is what lets guidance skip
    /// the live/frozen split tasks and features need: it owns nothing but
    /// `String`s and `PathBuf`s.
    fn guidance(&self) -> Vec<Guidance>;
}

impl FrozenGuidanceModuleLike for FrozenModule {
    fn guidance(&self) -> Vec<Guidance> {
        self.names()
            .filter_map(|symbol| {
                let owned = self.get(symbol.as_str()).ok()?;
                owned.value().downcast_ref::<Guidance>().cloned()
            })
            .collect()
    }
}
