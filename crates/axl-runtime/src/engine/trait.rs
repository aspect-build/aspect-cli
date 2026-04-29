use std::cell::Cell;
use std::fmt::{self, Display, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use allocative::Allocative;
use dupe::Dupe;

use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::dict::AllocDict;
use starlark::values::list::AllocList;
use starlark::values::none::NoneOr;
use starlark::values::typing::TypeCompiled;
use starlark::values::{
    AllocFrozenValue, AllocValue, Freeze, FreezeError, Freezer, FrozenHeap, FrozenValue, Heap,
    NoSerialize, ProvidesStaticType, StarlarkValue, Trace, Tracer, Value, ValueLike,
    starlark_value,
};
use starlark_map::small_map::SmallMap;

use super::names::validate_type_name;

static TRAIT_TYPE_ID: AtomicU64 = AtomicU64::new(0);

fn next_trait_type_id() -> u64 {
    TRAIT_TYPE_ID.fetch_add(1, Ordering::SeqCst)
}

/// A field definition for a trait, containing a type, optional default value, and optional description.
#[derive(Debug, Clone, ProvidesStaticType, Allocative)]
pub struct Attr<'v> {
    pub(crate) typ: TypeCompiled<Value<'v>>,
    pub(crate) typ_value: Value<'v>,
    pub(crate) default: Option<Value<'v>>,
    pub(crate) description: Option<String>,
}

impl<'v> fmt::Display for Attr<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.default {
            None => write!(f, "attr({})", self.typ),
            Some(d) => write!(f, "attr({}, default = {})", self.typ, d),
        }
    }
}

unsafe impl<'v> Trace<'v> for Attr<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.typ.trace(tracer);
        self.typ_value.trace(tracer);
        if let Some(ref mut d) = self.default {
            d.trace(tracer);
        }
    }
}

impl<'v> Attr<'v> {
    pub fn freeze(self, freezer: &Freezer) -> Result<FrozenAttr, FreezeError> {
        Ok(FrozenAttr {
            typ: self.typ.freeze(freezer)?,
            typ_value: self.typ_value.freeze(freezer)?,
            default: self.default.map(|d| d.freeze(freezer)).transpose()?,
            description: self.description,
        })
    }
}

/// A frozen field definition for a trait.
#[derive(Debug, Clone, ProvidesStaticType, Allocative)]
pub struct FrozenAttr {
    pub(crate) typ: TypeCompiled<FrozenValue>,
    pub(crate) typ_value: FrozenValue,
    pub(crate) default: Option<FrozenValue>,
    pub(crate) description: Option<String>,
}

impl fmt::Display for FrozenAttr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.default {
            None => write!(f, "attr({})", self.typ),
            Some(d) => write!(f, "attr({}, default = {})", self.typ, d),
        }
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ConfigAttrValue<'v> {
    pub(crate) typ: TypeCompiled<Value<'v>>,
    pub(crate) typ_value: Value<'v>,
    pub(crate) default: Option<Value<'v>>,
    pub(crate) description: Option<String>,
}

impl<'v> fmt::Display for ConfigAttrValue<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.default {
            None => write!(f, "attr({})", self.typ),
            Some(d) => write!(f, "attr({}, default = {})", self.typ, d),
        }
    }
}

unsafe impl<'v> Trace<'v> for ConfigAttrValue<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.typ.trace(tracer);
        self.typ_value.trace(tracer);
        if let Some(ref mut d) = self.default {
            d.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for ConfigAttrValue<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "attr")]
impl<'v> StarlarkValue<'v> for ConfigAttrValue<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }
}

/// Frozen version of ConfigAttrValue.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenConfigAttrValue {
    pub(crate) typ: TypeCompiled<FrozenValue>,
    pub(crate) typ_value: FrozenValue,
    pub(crate) default: Option<FrozenValue>,
    pub(crate) description: Option<String>,
}

impl fmt::Display for FrozenConfigAttrValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.default {
            None => write!(f, "attr({})", self.typ),
            Some(d) => write!(f, "attr({}, default = {})", self.typ, d),
        }
    }
}

unsafe impl<'v> Trace<'v> for FrozenConfigAttrValue {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

impl AllocFrozenValue for FrozenConfigAttrValue {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "attr")]
impl<'v> StarlarkValue<'v> for FrozenConfigAttrValue {
    type Canonical = ConfigAttrValue<'v>;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }
}

impl Freeze for ConfigAttrValue<'_> {
    type Frozen = FrozenConfigAttrValue;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        Ok(FrozenConfigAttrValue {
            typ: self.typ.freeze(freezer)?,
            typ_value: self.typ_value.freeze(freezer)?,
            default: self.default.map(|d| d.freeze(freezer)).transpose()?,
            description: self.description,
        })
    }
}

/// Deep-copy a default value if it's a mutable container (list or dict).
pub fn copy_default_value<'v>(value: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
    match value.get_type() {
        "list" => {
            let items: Vec<Value<'v>> = value.iterate(heap).map_err(|e| e.into_anyhow())?.collect();
            Ok(heap.alloc(AllocList(items)))
        }
        "dict" => {
            let keys: Vec<Value<'v>> = value.iterate(heap).map_err(|e| e.into_anyhow())?.collect();
            let items: Vec<(Value<'v>, Value<'v>)> = keys
                .into_iter()
                .map(|k| {
                    let v = value.at(k, heap).map_err(|e| e.into_anyhow())?;
                    Ok((k, v))
                })
                .collect::<anyhow::Result<_>>()?;
            Ok(heap.alloc(AllocDict(items)))
        }
        _ => Ok(value),
    }
}

/// Create fresh TypeCompiled values from field type values at runtime.
pub fn build_type_checkers<'v>(
    fields: impl Iterator<Item = Value<'v>>,
    heap: Heap<'v>,
) -> starlark::Result<Vec<TypeCompiled<Value<'v>>>> {
    fields
        .map(|typ_value| TypeCompiled::new(typ_value, heap).map_err(starlark::Error::new_other))
        .collect()
}

/// The type of a trait, created by `trait(field1=type1, field2=type2, ...)`.
/// Calling this type creates a `TraitInstance` instance.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TraitType<'v> {
    /// Unique identifier for this trait type
    pub(crate) id: u64,
    /// Name of the trait type (set when assigned to a variable)
    pub(crate) name: Option<String>,
    /// Fields with their types and optional defaults
    pub(crate) fields: SmallMap<String, Attr<'v>>,
}

impl<'v> Display for TraitType<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "trait[{}]", name),
            None => write!(f, "trait[anon]"),
        }
    }
}

unsafe impl<'v> Trace<'v> for TraitType<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for (_, field) in self.fields.iter_mut() {
            field.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for TraitType<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "trait")]
impl<'v> StarlarkValue<'v> for TraitType<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn export_as(
        &self,
        variable_name: &str,
        _eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        validate_type_name(variable_name, "trait")
            .map_err(|e| starlark::Error::new_other(anyhow::anyhow!(e)))?;
        // We use unsafe to mutate the name, which is safe because this is only
        // called during module loading.
        let this = self as *const Self as *mut Self;
        unsafe {
            (*this).name = Some(variable_name.to_string());
        }
        Ok(())
    }

    fn invoke(
        &self,
        _me: Value<'v>,
        args: &starlark::eval::Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        // Build fresh type checkers from the original type values
        let type_checkers =
            build_type_checkers(self.fields.values().map(|f| f.typ_value), eval.heap())?;

        // Parse the arguments according to our field definitions
        let mut values: Vec<Cell<Value<'v>>> = Vec::with_capacity(self.fields.len());

        // Get the named arguments
        args.no_positional_args(eval.heap())?;
        let kwargs = args.names_map()?;

        // Build values in field order
        for ((field_name, field), tc) in self.fields.iter().zip(type_checkers.iter()) {
            let value = if let Some(v) = kwargs.get(field_name.as_str()) {
                *v
            } else if let Some(default) = field.default {
                copy_default_value(default, eval.heap())?
            } else {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Missing required field `{}` for {}",
                    field_name,
                    self
                )));
            };

            // Type check the value using the fresh TypeCompiled
            if !tc.matches(value) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Field `{}` expected type `{}`, got `{}`",
                    field_name,
                    tc,
                    value.get_type()
                )));
            }

            values.push(Cell::new(value));
        }

        // Check for unexpected kwargs
        for (name, _) in kwargs.iter() {
            if !self.fields.contains_key(name.as_str()) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Unexpected field `{}` for {}",
                    name,
                    self
                )));
            }
        }

        let instance = TraitInstance {
            typ: _me,
            values: values.into_boxed_slice(),
            type_checkers: type_checkers.into_boxed_slice(),
        };
        Ok(eval.heap().alloc(instance))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(trait_type_methods)
    }
}

#[starlark_module]
fn trait_type_methods(_builder: &mut MethodsBuilder) {}

/// Frozen version of TraitType.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenTraitType {
    pub(crate) id: u64,
    pub(crate) name: Option<String>,
    pub(crate) fields: SmallMap<String, FrozenAttr>,
}

impl Display for FrozenTraitType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "trait[{}]", name),
            None => write!(f, "trait[anon]"),
        }
    }
}

unsafe impl<'v> Trace<'v> for FrozenTraitType {
    fn trace(&mut self, _tracer: &Tracer<'v>) {
        // Frozen values don't need tracing
    }
}

impl AllocFrozenValue for FrozenTraitType {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "trait")]
impl<'v> StarlarkValue<'v> for FrozenTraitType {
    type Canonical = TraitType<'v>;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn invoke(
        &self,
        _me: Value<'v>,
        args: &starlark::eval::Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        // Build fresh type checkers from the original type values
        let type_checkers = build_type_checkers(
            self.fields.values().map(|f| f.typ_value.to_value()),
            eval.heap(),
        )?;

        let mut values: Vec<Cell<Value<'v>>> = Vec::with_capacity(self.fields.len());

        args.no_positional_args(eval.heap())?;
        let kwargs = args.names_map()?;

        for ((field_name, field), tc) in self.fields.iter().zip(type_checkers.iter()) {
            let value = if let Some(v) = kwargs.get(field_name.as_str()) {
                *v
            } else if let Some(default) = field.default {
                copy_default_value(default.to_value(), eval.heap())?
            } else {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Missing required field `{}` for {}",
                    field_name,
                    self
                )));
            };

            // Type check using the fresh TypeCompiled
            if !tc.matches(value) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Field `{}` expected type `{}`, got `{}`",
                    field_name,
                    tc,
                    value.get_type()
                )));
            }

            values.push(Cell::new(value));
        }

        for (name, _) in kwargs.iter() {
            if !self.fields.contains_key(name.as_str()) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Unexpected field `{}` for {}",
                    name,
                    self
                )));
            }
        }

        let instance = TraitInstance {
            typ: _me,
            values: values.into_boxed_slice(),
            type_checkers: type_checkers.into_boxed_slice(),
        };
        Ok(eval.heap().alloc(instance))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(trait_type_methods)
    }
}

impl Freeze for TraitType<'_> {
    type Frozen = FrozenTraitType;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let mut frozen_fields = SmallMap::with_capacity(self.fields.len());
        for (name, field) in self.fields.into_iter() {
            frozen_fields.insert(name, field.freeze(freezer)?);
        }
        Ok(FrozenTraitType {
            id: self.id,
            name: self.name,
            fields: frozen_fields,
        })
    }
}

/// An instance of a trait type, containing field values.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TraitInstance<'v> {
    /// The trait type this instance belongs to
    pub(crate) typ: Value<'v>,
    /// Field values in the same order as the type's field definitions (mutable via Cell)
    #[allocative(skip)]
    pub(crate) values: Box<[Cell<Value<'v>>]>,
    /// Fresh type checkers created at construction time for runtime type checking.
    /// These are re-derived from the field type values to avoid issues with frozen TypeCompiled.
    #[allocative(skip)]
    pub(crate) type_checkers: Box<[TypeCompiled<Value<'v>>]>,
}

impl<'v> Display for TraitInstance<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.typ)?;
        if let Some(trait_type) = self.typ.downcast_ref::<TraitType>() {
            let mut first = true;
            for ((name, _), value) in trait_type.fields.iter().zip(self.values.iter()) {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                write!(f, "{}={}", name, value.get())?;
            }
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenTraitType>() {
            let mut first = true;
            for ((name, _), value) in frozen_type.fields.iter().zip(self.values.iter()) {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                write!(f, "{}={}", name, value.get())?;
            }
        }
        write!(f, ")")
    }
}

unsafe impl<'v> Trace<'v> for TraitInstance<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.typ.trace(tracer);
        for cell in self.values.iter() {
            let mut v = cell.get();
            v.trace(tracer);
            cell.set(v);
        }
        for tc in self.type_checkers.iter_mut() {
            tc.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for TraitInstance<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> TraitInstance<'v> {
    fn get_field_names(&self) -> Vec<&str> {
        if let Some(trait_type) = self.typ.downcast_ref::<TraitType>() {
            trait_type.fields.keys().map(|s| s.as_str()).collect()
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenTraitType>() {
            frozen_type.fields.keys().map(|s| s.as_str()).collect()
        } else {
            vec![]
        }
    }
}

#[starlark_value(type = "trait")]
impl<'v> StarlarkValue<'v> for TraitInstance<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        if let Some(trait_type) = self.typ.downcast_ref::<TraitType>() {
            if let Some(idx) = trait_type.fields.get_index_of(attribute) {
                return Some(self.values[idx].get());
            }
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenTraitType>() {
            if let Some(idx) = frozen_type.fields.get_index_of(attribute) {
                return Some(self.values[idx].get());
            }
        }
        None
    }

    fn set_attr(&self, attribute: &str, value: Value<'v>) -> starlark::Result<()> {
        // Get field index
        let idx = if let Some(trait_type) = self.typ.downcast_ref::<TraitType>() {
            trait_type.fields.get_index_of(attribute)
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenTraitType>() {
            frozen_type.fields.get_index_of(attribute)
        } else {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "Invalid trait type"
            )));
        };

        let idx = match idx {
            Some(idx) => idx,
            None => {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Trait {} has no field `{}`",
                    self.typ,
                    attribute
                )));
            }
        };

        // Type check using the fresh type checker created at construction time
        let tc = &self.type_checkers[idx];
        if !tc.matches(value) {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "Field `{}` expected type `{}`, got `{}`",
                attribute,
                tc,
                value.get_type()
            )));
        }

        // Set the value
        self.values[idx].set(value);
        Ok(())
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        if let Some(trait_type) = self.typ.downcast_ref::<TraitType>() {
            trait_type.fields.contains_key(attribute)
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenTraitType>() {
            frozen_type.fields.contains_key(attribute)
        } else {
            false
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        self.get_field_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        if let Some(other_instance) = other.downcast_ref::<TraitInstance>() {
            // Check that they have the same trait type
            let self_id = self
                .typ
                .downcast_ref::<TraitType>()
                .map(|t| t.id)
                .or_else(|| self.typ.downcast_ref::<FrozenTraitType>().map(|t| t.id));
            let other_id = other_instance
                .typ
                .downcast_ref::<TraitType>()
                .map(|t| t.id)
                .or_else(|| {
                    other_instance
                        .typ
                        .downcast_ref::<FrozenTraitType>()
                        .map(|t| t.id)
                });

            if self_id != other_id {
                return Ok(false);
            }

            // Compare all values
            if self.values.len() != other_instance.values.len() {
                return Ok(false);
            }
            for (a, b) in self.values.iter().zip(other_instance.values.iter()) {
                if !a.get().equals(b.get())? {
                    return Ok(false);
                }
            }
            Ok(true)
        } else if let Some(other_frozen) = other.downcast_ref::<FrozenTraitInstance>() {
            let self_id = self
                .typ
                .downcast_ref::<TraitType>()
                .map(|t| t.id)
                .or_else(|| self.typ.downcast_ref::<FrozenTraitType>().map(|t| t.id));
            let other_id = other_frozen
                .typ
                .downcast_ref::<FrozenTraitType>()
                .map(|t| t.id);

            if self_id != other_id {
                return Ok(false);
            }

            if self.values.len() != other_frozen.values.len() {
                return Ok(false);
            }
            for (a, b) in self.values.iter().zip(other_frozen.values.iter()) {
                if !a.get().equals(b.to_value())? {
                    return Ok(false);
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

// -----------------------------------------------------------------------------
// FrozenTraitInstance
// -----------------------------------------------------------------------------

/// Frozen version of TraitInstance.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenTraitInstance {
    pub(crate) typ: FrozenValue,
    pub(crate) values: Box<[FrozenValue]>,
}

impl Display for FrozenTraitInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.typ)?;
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenTraitType>() {
            let mut first = true;
            for ((name, _), value) in frozen_type.fields.iter().zip(self.values.iter()) {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                write!(f, "{}={}", name, value)?;
            }
        }
        write!(f, ")")
    }
}

unsafe impl<'v> Trace<'v> for FrozenTraitInstance {
    fn trace(&mut self, _tracer: &Tracer<'v>) {
        // Frozen values don't need tracing
    }
}

impl AllocFrozenValue for FrozenTraitInstance {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "trait")]
impl<'v> StarlarkValue<'v> for FrozenTraitInstance {
    type Canonical = TraitInstance<'v>;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenTraitType>() {
            if let Some(idx) = frozen_type.fields.get_index_of(attribute) {
                return Some(self.values[idx].to_value());
            }
        }
        None
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenTraitType>() {
            frozen_type.fields.contains_key(attribute)
        } else {
            false
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenTraitType>() {
            frozen_type.fields.keys().map(|s| s.to_string()).collect()
        } else {
            vec![]
        }
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        if let Some(other_frozen) = other.downcast_ref::<FrozenTraitInstance>() {
            let self_id = self.typ.downcast_ref::<FrozenTraitType>().map(|t| t.id);
            let other_id = other_frozen
                .typ
                .downcast_ref::<FrozenTraitType>()
                .map(|t| t.id);

            if self_id != other_id {
                return Ok(false);
            }

            if self.values.len() != other_frozen.values.len() {
                return Ok(false);
            }
            for (a, b) in self.values.iter().zip(other_frozen.values.iter()) {
                if !a.to_value().equals(b.to_value())? {
                    return Ok(false);
                }
            }
            Ok(true)
        } else if let Some(other_instance) = other.downcast_ref::<TraitInstance>() {
            let self_id = self.typ.downcast_ref::<FrozenTraitType>().map(|t| t.id);
            let other_id = other_instance
                .typ
                .downcast_ref::<TraitType>()
                .map(|t| t.id)
                .or_else(|| {
                    other_instance
                        .typ
                        .downcast_ref::<FrozenTraitType>()
                        .map(|t| t.id)
                });

            if self_id != other_id {
                return Ok(false);
            }

            if self.values.len() != other_instance.values.len() {
                return Ok(false);
            }
            for (a, b) in self.values.iter().zip(other_instance.values.iter()) {
                if !a.to_value().equals(b.get())? {
                    return Ok(false);
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl Freeze for TraitInstance<'_> {
    type Frozen = FrozenTraitInstance;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let typ = self.typ.freeze(freezer)?;
        let values: Result<Vec<_>, _> = self
            .values
            .iter()
            .map(|v| v.get().freeze(freezer))
            .collect();
        Ok(FrozenTraitInstance {
            typ,
            values: values?.into_boxed_slice(),
        })
    }
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Construct a default instance of a trait type using only the heap.
pub fn construct_default_instance<'v>(
    type_val: Value<'v>,
    heap: Heap<'v>,
) -> starlark::Result<Value<'v>> {
    if let Some(tt) = type_val.downcast_ref::<TraitType<'v>>() {
        let type_checkers = build_type_checkers(tt.fields.values().map(|f| f.typ_value), heap)?;
        let mut values = Vec::with_capacity(tt.fields.len());
        for ((field_name, field), tc) in tt.fields.iter().zip(type_checkers.iter()) {
            let value = match field.default {
                Some(d) => copy_default_value(d, heap).map_err(starlark::Error::new_other)?,
                None => {
                    return Err(starlark::Error::new_other(anyhow::anyhow!(
                        "Trait field `{}` has no default; cannot construct default instance of {}",
                        field_name,
                        type_val
                    )));
                }
            };
            if !tc.matches(value) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Field `{}` expected type `{}`, got `{}`",
                    field_name,
                    tc,
                    value.get_type()
                )));
            }
            values.push(Cell::new(value));
        }
        Ok(heap.alloc(TraitInstance {
            typ: type_val,
            values: values.into_boxed_slice(),
            type_checkers: type_checkers.into_boxed_slice(),
        }))
    } else if let Some(ft) = type_val.downcast_ref::<FrozenTraitType>() {
        let type_checkers =
            build_type_checkers(ft.fields.values().map(|f| f.typ_value.to_value()), heap)?;
        let mut values = Vec::with_capacity(ft.fields.len());
        for ((field_name, field), tc) in ft.fields.iter().zip(type_checkers.iter()) {
            let value = match field.default {
                Some(d) => {
                    copy_default_value(d.to_value(), heap).map_err(starlark::Error::new_other)?
                }
                None => {
                    return Err(starlark::Error::new_other(anyhow::anyhow!(
                        "Trait field `{}` has no default; cannot construct default instance of {}",
                        field_name,
                        type_val
                    )));
                }
            };
            if !tc.matches(value) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Field `{}` expected type `{}`, got `{}`",
                    field_name,
                    tc,
                    value.get_type()
                )));
            }
            values.push(Cell::new(value));
        }
        Ok(heap.alloc(TraitInstance {
            typ: type_val,
            values: values.into_boxed_slice(),
            type_checkers: type_checkers.into_boxed_slice(),
        }))
    } else {
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "Value is not a trait type: {}",
            type_val
        )))
    }
}

/// Extract the trait type ID from a Value that is either a TraitType or FrozenTraitType.
pub fn extract_trait_type_id(value: Value) -> Option<u64> {
    if let Some(ft) = value.downcast_ref::<TraitType>() {
        Some(ft.id)
    } else if let Some(ft) = value.downcast_ref::<FrozenTraitType>() {
        Some(ft.id)
    } else {
        None
    }
}

// -----------------------------------------------------------------------------
// Global functions: trait() and attr()
// -----------------------------------------------------------------------------

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    /// Creates a trait type — a shared configuration object that tasks opt into.
    ///
    /// ## Naming
    ///
    /// Traits must be exported as **CamelCase** (`MyConfig`, `BazelTrait`). This is
    /// enforced at definition time.
    ///
    /// ## Fields
    ///
    /// Each field must be an `attr()` definition with a `default` value. The default is used
    /// to construct the initial trait instance lazily on first access — there is no mechanism
    /// to inject values before that construction, so all fields must have defaults.
    ///
    /// ## Example
    ///
    /// ```starlark
    /// BazelTrait = trait(
    ///     extra_flags    = attr(list[str], default = [], description = "Extra Bazel flags for every build"),
    ///     profile_upload = attr(bool,      default = False, description = "Upload Bazel profile after build"),
    /// )
    /// ```
    fn r#trait<'v>(
        #[starlark(kwargs)] kwargs: SmallMap<&str, Value<'v>>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<TraitType<'v>> {
        let mut fields = SmallMap::with_capacity(kwargs.len());

        for (name, value) in kwargs.into_iter() {
            let field = if let Some(attr_value) = value.downcast_ref::<ConfigAttrValue>() {
                // It's already an attr() definition
                Attr {
                    typ: attr_value.typ.dupe(),
                    typ_value: attr_value.typ_value,
                    default: attr_value.default,
                    description: attr_value.description.clone(),
                }
            } else {
                // It's a type, convert to an attr without default
                let typ = TypeCompiled::new(value, eval.heap())?;
                Attr {
                    typ,
                    typ_value: value,
                    default: None,
                    description: None,
                }
            };
            fields.insert(name.to_string(), field);
        }

        Ok(TraitType {
            id: next_trait_type_id(),
            name: None,
            fields,
        })
    }

    /// Creates a field definition for a trait, with a type, optional default value,
    /// and optional description.
    ///
    /// `default` must match the declared type. Mutable defaults (lists, dicts) are deep-copied
    /// when a trait instance is created, so each instance gets its own independent copy.
    ///
    /// Example:
    /// ```starlark
    /// BazelTrait = trait(host=str, port=attr(int, default = 80))
    /// r = BazelTrait(host="localhost")  # port defaults to 80
    /// ```
    fn attr<'v>(
        #[starlark(require = pos)] typ: Value<'v>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named, default = NoneOr::None)] description: NoneOr<String>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<ConfigAttrValue<'v>> {
        let compiled_type = TypeCompiled::new(typ, eval.heap())?;
        let description = description.into_option();

        if let Some(d) = default {
            if !compiled_type.matches(d) {
                return Err(anyhow::anyhow!(
                    "Default value `{}` does not match attr type `{}`",
                    d,
                    compiled_type
                ));
            }
        }

        Ok(ConfigAttrValue {
            typ: compiled_type,
            typ_value: typ,
            default,
            description,
        })
    }
}
