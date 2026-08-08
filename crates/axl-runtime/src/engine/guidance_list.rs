//! `ctx.guidance` — the collection a `config.axl` adds topics to.
//!
//! Much simpler than [`super::feature_map::FeatureMap`] because [`Guidance`]
//! holds no `Value<'v>`: there is nothing for the garbage collector to trace
//! and nothing to thaw, so the list stores the structs directly and freezing is
//! a move.

use std::cell::RefCell;
use std::fmt::{self, Display, Write};

use allocative::Allocative;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, Heap, NoSerialize, ProvidesStaticType, StarlarkValue,
    Trace, Tracer, Value, ValueLike, none::NoneType, starlark_value,
};

use super::guidance::Guidance;

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct GuidanceList {
    #[allocative(skip)]
    entries: RefCell<Vec<Guidance>>,
}

impl GuidanceList {
    pub fn new() -> Self {
        GuidanceList {
            entries: RefCell::new(Vec::new()),
        }
    }

    pub fn insert(&self, topic: Guidance) {
        self.entries.borrow_mut().push(topic);
    }

    /// Topics in declaration order, duplicates included. De-duplication is the
    /// reader's job — see `MultiPhaseEval::guidance`.
    pub fn values(&self) -> Vec<Guidance> {
        self.entries.borrow().clone()
    }
}

impl Default for GuidanceList {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for GuidanceList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GuidanceList([")?;
        for (i, t) in self.entries.borrow().iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{t}")?;
        }
        write!(f, "])")
    }
}

// No heap values inside, so tracing has nothing to visit.
unsafe impl<'v> Trace<'v> for GuidanceList {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

impl<'v> AllocValue<'v> for GuidanceList {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl Freeze for GuidanceList {
    type Frozen = FrozenGuidanceList;

    fn freeze(self, _freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        Ok(FrozenGuidanceList {
            entries: self.entries.into_inner(),
        })
    }
}

#[starlark_value(type = "GuidanceList")]
impl<'v> StarlarkValue<'v> for GuidanceList {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{self}").unwrap();
    }

    fn get_methods() -> Option<&'static starlark::environment::Methods> {
        static RES: starlark::environment::MethodsStatic =
            starlark::environment::MethodsStatic::new();
        RES.methods(guidance_list_methods)
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenGuidanceList {
    #[allocative(skip)]
    entries: Vec<Guidance>,
}

impl Display for FrozenGuidanceList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GuidanceList([{} topics])", self.entries.len())
    }
}

unsafe impl<'v> Trace<'v> for FrozenGuidanceList {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

starlark::starlark_simple_value!(FrozenGuidanceList);

#[starlark_value(type = "GuidanceList")]
impl<'v> StarlarkValue<'v> for FrozenGuidanceList {
    type Canonical = GuidanceList;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{self}").unwrap();
    }
}

#[starlark::starlark_module]
fn guidance_list_methods(registry: &mut starlark::environment::MethodsBuilder) {
    /// Adds a guidance topic from a `config.axl`.
    ///
    /// Declaring an id that already exists replaces it — that is the point, and
    /// it is how a repo overrides guidance a module shipped. `defined_in` in
    /// `aspect describe` reports which declaration won, so an accidental
    /// collision is visible rather than silent.
    ///
    /// ```starlark
    /// ctx.guidance.add(guidance(
    ///     id = "bazelify-go",
    ///     title = "Bazel-ify a Go repo (our conventions)",
    ///     body_file = "./our-bazelify-go.md",
    /// ))
    /// ```
    fn add<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] topic: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let list = this
            .downcast_ref::<GuidanceList>()
            .ok_or_else(|| anyhow::anyhow!("ctx.guidance is not a GuidanceList"))?;
        let topic = topic.downcast_ref::<Guidance>().ok_or_else(|| {
            anyhow::anyhow!(
                "ctx.guidance.add expects a guidance(...) value, got '{}'",
                topic.get_type()
            )
        })?;
        list.insert(topic.clone());
        Ok(NoneType)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::engine::guidance::GuidanceStability;

    fn topic(id: &str, title: &str) -> Guidance {
        Guidance::new(
            id.to_owned(),
            title.to_owned(),
            String::new(),
            PathBuf::from("/repo/body.md"),
            vec![],
            GuidanceStability::Stable,
            PathBuf::from("/repo/topics.axl"),
        )
    }

    /// The list keeps duplicates; `MultiPhaseEval::guidance` resolves them
    /// last-wins. Collapsing here instead would lose the declaration order that
    /// decides which one that is.
    #[test]
    fn insert_preserves_declaration_order_including_duplicate_ids() {
        let list = GuidanceList::new();
        list.insert(topic("bazelify-go", "shipped"));
        list.insert(topic("other", "other"));
        list.insert(topic("bazelify-go", "repo override"));

        let got = list.values();
        assert_eq!(got.len(), 3, "duplicates survive for the reader to resolve");
        assert_eq!(got[0].title(), "shipped");
        assert_eq!(got[2].title(), "repo override");
    }
}
