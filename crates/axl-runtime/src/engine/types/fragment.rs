use std::cell::Cell;
use std::fmt::{self, Display, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use allocative::Allocative;
use dupe::Dupe;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::dict::AllocDict;
use starlark::values::list::AllocList;
use starlark::values::typing::TypeCompiled;
use starlark::values::{
    AllocFrozenValue, AllocValue, Freeze, FreezeError, Freezer, FrozenHeap, FrozenValue, Heap,
    NoSerialize, ProvidesStaticType, StarlarkValue, Trace, Tracer, Value, ValueLike,
    starlark_value,
};
use starlark_map::small_map::SmallMap;

static FRAGMENT_TYPE_ID: AtomicU64 = AtomicU64::new(0);

fn next_fragment_type_id() -> u64 {
    FRAGMENT_TYPE_ID.fetch_add(1, Ordering::SeqCst)
}

// -----------------------------------------------------------------------------
// Field
// -----------------------------------------------------------------------------

/// A field definition for a fragment, containing a type and optional default value.
#[derive(Debug, Clone, ProvidesStaticType, Allocative)]
pub struct Field<'v> {
    pub(crate) typ: TypeCompiled<Value<'v>>,
    pub(crate) typ_value: Value<'v>,
    pub(crate) default: Option<Value<'v>>,
}

impl<'v> Display for Field<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.default {
            None => write!(f, "field({})", self.typ),
            Some(d) => write!(f, "field({}, {})", self.typ, d),
        }
    }
}

unsafe impl<'v> Trace<'v> for Field<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.typ.trace(tracer);
        self.typ_value.trace(tracer);
        if let Some(ref mut d) = self.default {
            d.trace(tracer);
        }
    }
}

impl<'v> Field<'v> {
    pub fn freeze(self, freezer: &Freezer) -> Result<FrozenField, FreezeError> {
        Ok(FrozenField {
            typ: self.typ.freeze(freezer)?,
            typ_value: self.typ_value.freeze(freezer)?,
            default: self.default.map(|d| d.freeze(freezer)).transpose()?,
        })
    }
}

/// A frozen field definition.
#[derive(Debug, Clone, ProvidesStaticType, Allocative)]
pub struct FrozenField {
    pub(crate) typ: TypeCompiled<FrozenValue>,
    pub(crate) typ_value: FrozenValue,
    pub(crate) default: Option<FrozenValue>,
}

impl Display for FrozenField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.default {
            None => write!(f, "field({})", self.typ),
            Some(d) => write!(f, "field({}, {})", self.typ, d),
        }
    }
}

// -----------------------------------------------------------------------------
// FieldValue - a wrapper for field() function return
// -----------------------------------------------------------------------------

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FieldValue<'v> {
    pub(crate) typ: TypeCompiled<Value<'v>>,
    pub(crate) typ_value: Value<'v>,
    pub(crate) default: Option<Value<'v>>,
}

impl<'v> Display for FieldValue<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.default {
            None => write!(f, "field({})", self.typ),
            Some(d) => write!(f, "field({}, {})", self.typ, d),
        }
    }
}

unsafe impl<'v> Trace<'v> for FieldValue<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.typ.trace(tracer);
        self.typ_value.trace(tracer);
        if let Some(ref mut d) = self.default {
            d.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for FieldValue<'v> {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "field")]
impl<'v> StarlarkValue<'v> for FieldValue<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }
}

/// Frozen version of FieldValue.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenFieldValue {
    pub(crate) typ: TypeCompiled<FrozenValue>,
    pub(crate) typ_value: FrozenValue,
    pub(crate) default: Option<FrozenValue>,
}

impl Display for FrozenFieldValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.default {
            None => write!(f, "field({})", self.typ),
            Some(d) => write!(f, "field({}, {})", self.typ, d),
        }
    }
}

unsafe impl<'v> Trace<'v> for FrozenFieldValue {
    fn trace(&mut self, _tracer: &Tracer<'v>) {
        // Frozen values don't need tracing
    }
}

impl AllocFrozenValue for FrozenFieldValue {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "field")]
impl<'v> StarlarkValue<'v> for FrozenFieldValue {
    type Canonical = FieldValue<'v>;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }
}

impl Freeze for FieldValue<'_> {
    type Frozen = FrozenFieldValue;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        Ok(FrozenFieldValue {
            typ: self.typ.freeze(freezer)?,
            typ_value: self.typ_value.freeze(freezer)?,
            default: self.default.map(|d| d.freeze(freezer)).transpose()?,
        })
    }
}

/// Deep-copy a default value if it's a mutable container (list or dict).
/// This ensures each fragment instance gets its own mutable copy rather than
/// sharing the (potentially frozen) default.
fn copy_default_value<'v>(value: Value<'v>, heap: &'v Heap) -> starlark::Result<Value<'v>> {
    match value.get_type() {
        "list" => {
            let items: Vec<Value<'v>> = value.iterate(heap)?.collect();
            Ok(heap.alloc(AllocList(items)))
        }
        "dict" => {
            let keys: Vec<Value<'v>> = value.iterate(heap)?.collect();
            let items: Vec<(Value<'v>, Value<'v>)> = keys
                .into_iter()
                .map(|k| {
                    let v = value.at(k, heap)?;
                    Ok((k, v))
                })
                .collect::<starlark::Result<_>>()?;
            Ok(heap.alloc(AllocDict(items)))
        }
        _ => Ok(value),
    }
}

/// Create fresh TypeCompiled values from field type values at runtime.
/// This ensures type checking works correctly for types like starlark Records
/// whose frozen TypeCompiled matchers may not function properly.
fn build_type_checkers<'v>(
    fields: impl Iterator<Item = Value<'v>>,
    heap: &'v Heap,
) -> starlark::Result<Vec<TypeCompiled<Value<'v>>>> {
    fields
        .map(|typ_value| TypeCompiled::new(typ_value, heap).map_err(starlark::Error::new_other))
        .collect()
}

// -----------------------------------------------------------------------------
// FragmentType
// -----------------------------------------------------------------------------

/// The type of a fragment, created by `fragment(field1=type1, field2=type2, ...)`.
/// Calling this type creates a `FragmentInstance` instance.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FragmentType<'v> {
    /// Unique identifier for this fragment type
    pub(crate) id: u64,
    /// Name of the fragment type (set when assigned to a variable)
    pub(crate) name: Option<String>,
    /// Fields with their types and optional defaults
    pub(crate) fields: SmallMap<String, Field<'v>>,
}

impl<'v> Display for FragmentType<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "fragment[{}]", name),
            None => write!(f, "fragment[anon]"),
        }
    }
}

unsafe impl<'v> Trace<'v> for FragmentType<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for (_, field) in self.fields.iter_mut() {
            field.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for FragmentType<'v> {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "fragment")]
impl<'v> StarlarkValue<'v> for FragmentType<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn export_as(
        &self,
        variable_name: &str,
        _eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        // This is called when the fragment type is assigned to a variable.
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

        let instance = FragmentInstance {
            typ: _me,
            values: values.into_boxed_slice(),
            type_checkers: type_checkers.into_boxed_slice(),
        };
        Ok(eval.heap().alloc(instance))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(fragment_type_methods)
    }
}

#[starlark_module]
fn fragment_type_methods(_builder: &mut MethodsBuilder) {}

// -----------------------------------------------------------------------------
// FrozenFragmentType
// -----------------------------------------------------------------------------

/// Frozen version of FragmentType.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenFragmentType {
    pub(crate) id: u64,
    pub(crate) name: Option<String>,
    pub(crate) fields: SmallMap<String, FrozenField>,
}

impl Display for FrozenFragmentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "fragment[{}]", name),
            None => write!(f, "fragment[anon]"),
        }
    }
}

unsafe impl<'v> Trace<'v> for FrozenFragmentType {
    fn trace(&mut self, _tracer: &Tracer<'v>) {
        // Frozen values don't need tracing
    }
}

impl AllocFrozenValue for FrozenFragmentType {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "fragment")]
impl<'v> StarlarkValue<'v> for FrozenFragmentType {
    type Canonical = FragmentType<'v>;

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

        let instance = FragmentInstance {
            typ: _me,
            values: values.into_boxed_slice(),
            type_checkers: type_checkers.into_boxed_slice(),
        };
        Ok(eval.heap().alloc(instance))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(fragment_type_methods)
    }
}

impl Freeze for FragmentType<'_> {
    type Frozen = FrozenFragmentType;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let mut frozen_fields = SmallMap::with_capacity(self.fields.len());
        for (name, field) in self.fields.into_iter() {
            frozen_fields.insert(name, field.freeze(freezer)?);
        }
        Ok(FrozenFragmentType {
            id: self.id,
            name: self.name,
            fields: frozen_fields,
        })
    }
}

// -----------------------------------------------------------------------------
// FragmentInstance
// -----------------------------------------------------------------------------

/// An instance of a fragment type, containing field values.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FragmentInstance<'v> {
    /// The fragment type this instance belongs to
    pub(crate) typ: Value<'v>,
    /// Field values in the same order as the type's field definitions (mutable via Cell)
    #[allocative(skip)]
    pub(crate) values: Box<[Cell<Value<'v>>]>,
    /// Fresh type checkers created at construction time for runtime type checking.
    /// These are re-derived from the field type values to avoid issues with frozen TypeCompiled.
    #[allocative(skip)]
    pub(crate) type_checkers: Box<[TypeCompiled<Value<'v>>]>,
}

impl<'v> Display for FragmentInstance<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.typ)?;
        if let Some(frag_type) = self.typ.downcast_ref::<FragmentType>() {
            let mut first = true;
            for ((name, _), value) in frag_type.fields.iter().zip(self.values.iter()) {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                write!(f, "{}={}", name, value.get())?;
            }
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFragmentType>() {
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

unsafe impl<'v> Trace<'v> for FragmentInstance<'v> {
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

impl<'v> AllocValue<'v> for FragmentInstance<'v> {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> FragmentInstance<'v> {
    fn get_field_names(&self) -> Vec<&str> {
        if let Some(frag_type) = self.typ.downcast_ref::<FragmentType>() {
            frag_type.fields.keys().map(|s| s.as_str()).collect()
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFragmentType>() {
            frozen_type.fields.keys().map(|s| s.as_str()).collect()
        } else {
            vec![]
        }
    }
}

#[starlark_value(type = "fragment")]
impl<'v> StarlarkValue<'v> for FragmentInstance<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn get_attr(&self, attribute: &str, _heap: &'v Heap) -> Option<Value<'v>> {
        if let Some(frag_type) = self.typ.downcast_ref::<FragmentType>() {
            if let Some(idx) = frag_type.fields.get_index_of(attribute) {
                return Some(self.values[idx].get());
            }
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFragmentType>() {
            if let Some(idx) = frozen_type.fields.get_index_of(attribute) {
                return Some(self.values[idx].get());
            }
        }
        None
    }

    fn set_attr(&self, attribute: &str, value: Value<'v>) -> starlark::Result<()> {
        // Get field index
        let idx = if let Some(frag_type) = self.typ.downcast_ref::<FragmentType>() {
            frag_type.fields.get_index_of(attribute)
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFragmentType>() {
            frozen_type.fields.get_index_of(attribute)
        } else {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "Invalid fragment type"
            )));
        };

        let idx = match idx {
            Some(idx) => idx,
            None => {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Fragment {} has no field `{}`",
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

    fn has_attr(&self, attribute: &str, _heap: &'v Heap) -> bool {
        if let Some(frag_type) = self.typ.downcast_ref::<FragmentType>() {
            frag_type.fields.contains_key(attribute)
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFragmentType>() {
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
        if let Some(other_instance) = other.downcast_ref::<FragmentInstance>() {
            // Check that they have the same fragment type
            let self_id = self
                .typ
                .downcast_ref::<FragmentType>()
                .map(|t| t.id)
                .or_else(|| self.typ.downcast_ref::<FrozenFragmentType>().map(|t| t.id));
            let other_id = other_instance
                .typ
                .downcast_ref::<FragmentType>()
                .map(|t| t.id)
                .or_else(|| {
                    other_instance
                        .typ
                        .downcast_ref::<FrozenFragmentType>()
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
        } else if let Some(other_frozen) = other.downcast_ref::<FrozenFragmentInstance>() {
            let self_id = self
                .typ
                .downcast_ref::<FragmentType>()
                .map(|t| t.id)
                .or_else(|| self.typ.downcast_ref::<FrozenFragmentType>().map(|t| t.id));
            let other_id = other_frozen
                .typ
                .downcast_ref::<FrozenFragmentType>()
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
// FrozenFragmentInstance
// -----------------------------------------------------------------------------

/// Frozen version of FragmentInstance.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenFragmentInstance {
    pub(crate) typ: FrozenValue,
    pub(crate) values: Box<[FrozenValue]>,
}

impl Display for FrozenFragmentInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.typ)?;
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFragmentType>() {
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

unsafe impl<'v> Trace<'v> for FrozenFragmentInstance {
    fn trace(&mut self, _tracer: &Tracer<'v>) {
        // Frozen values don't need tracing
    }
}

impl AllocFrozenValue for FrozenFragmentInstance {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "fragment")]
impl<'v> StarlarkValue<'v> for FrozenFragmentInstance {
    type Canonical = FragmentInstance<'v>;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn get_attr(&self, attribute: &str, _heap: &'v Heap) -> Option<Value<'v>> {
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFragmentType>() {
            if let Some(idx) = frozen_type.fields.get_index_of(attribute) {
                return Some(self.values[idx].to_value());
            }
        }
        None
    }

    fn has_attr(&self, attribute: &str, _heap: &'v Heap) -> bool {
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFragmentType>() {
            frozen_type.fields.contains_key(attribute)
        } else {
            false
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenFragmentType>() {
            frozen_type.fields.keys().map(|s| s.to_string()).collect()
        } else {
            vec![]
        }
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        if let Some(other_frozen) = other.downcast_ref::<FrozenFragmentInstance>() {
            let self_id = self.typ.downcast_ref::<FrozenFragmentType>().map(|t| t.id);
            let other_id = other_frozen
                .typ
                .downcast_ref::<FrozenFragmentType>()
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
        } else if let Some(other_instance) = other.downcast_ref::<FragmentInstance>() {
            let self_id = self.typ.downcast_ref::<FrozenFragmentType>().map(|t| t.id);
            let other_id = other_instance
                .typ
                .downcast_ref::<FragmentType>()
                .map(|t| t.id)
                .or_else(|| {
                    other_instance
                        .typ
                        .downcast_ref::<FrozenFragmentType>()
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

impl Freeze for FragmentInstance<'_> {
    type Frozen = FrozenFragmentInstance;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let typ = self.typ.freeze(freezer)?;
        let values: Result<Vec<_>, _> = self
            .values
            .iter()
            .map(|v| v.get().freeze(freezer))
            .collect();
        Ok(FrozenFragmentInstance {
            typ,
            values: values?.into_boxed_slice(),
        })
    }
}

// -----------------------------------------------------------------------------
// Helper: extract fragment type ID from a Value
// -----------------------------------------------------------------------------

/// Extract the fragment type ID from a Value that is either a FragmentType or FrozenFragmentType.
pub fn extract_fragment_type_id(value: Value) -> Option<u64> {
    if let Some(ft) = value.downcast_ref::<FragmentType>() {
        Some(ft.id)
    } else if let Some(ft) = value.downcast_ref::<FrozenFragmentType>() {
        Some(ft.id)
    } else {
        None
    }
}

// -----------------------------------------------------------------------------
// Global functions
// -----------------------------------------------------------------------------

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    /// Creates a fragment type with the given fields.
    ///
    /// Each field can be a bare type (required, no default) or an `attr()`
    /// definition (with type and optional default).
    ///
    /// Mutable defaults (lists, dicts) are deep-copied per instance, so each
    /// instance gets its own independent copy. No `default_factory` needed.
    ///
    /// Example:
    /// ```starlark
    /// BazelFragment = fragment(
    ///     extra_flags = attr(list[str], []),
    ///     extra_startup_flags = attr(list[str], []),
    /// )
    /// ```
    fn fragment<'v>(
        #[starlark(kwargs)] kwargs: SmallMap<&str, Value<'v>>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<FragmentType<'v>> {
        let mut fields = SmallMap::with_capacity(kwargs.len());

        for (name, value) in kwargs.into_iter() {
            let field = if let Some(field_value) = value.downcast_ref::<FieldValue>() {
                // It's already a field() definition
                Field {
                    typ: field_value.typ.dupe(),
                    typ_value: field_value.typ_value,
                    default: field_value.default,
                }
            } else {
                // It's a type, convert to a field without default
                let typ = TypeCompiled::new(value, eval.heap())?;
                Field {
                    typ,
                    typ_value: value,
                    default: None,
                }
            };
            fields.insert(name.to_string(), field);
        }

        Ok(FragmentType {
            id: next_fragment_type_id(),
            name: None,
            fields,
        })
    }

    /// Creates a field definition with a type and optional default value.
    ///
    /// Mutable defaults (lists, dicts) are deep-copied when a fragment instance is
    /// created, so each instance gets its own independent copy.
    ///
    /// Example:
    /// ```starlark
    /// BazelFragment = fragment(host=str, port=attr(int, 80))
    /// r = BazelFragment(host="localhost")  # port defaults to 80
    /// ```
    fn attr<'v>(
        #[starlark(require = pos)] typ: Value<'v>,
        #[starlark(require = pos)] default: Option<Value<'v>>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<FieldValue<'v>> {
        let compiled_type = TypeCompiled::new(typ, eval.heap())?;

        // Validate that the default matches the type if provided
        if let Some(d) = default {
            if !compiled_type.matches(d) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Default value `{}` does not match field type `{}`",
                    d,
                    compiled_type
                )));
            }
        }

        Ok(FieldValue {
            typ: compiled_type,
            typ_value: typ,
            default,
        })
    }
}
