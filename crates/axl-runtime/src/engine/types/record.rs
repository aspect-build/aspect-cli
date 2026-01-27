use std::cell::Cell;
use std::fmt::{self, Display, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use allocative::Allocative;
use dupe::Dupe;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::typing::TypeCompiled;
use starlark::values::{
    starlark_value, AllocFrozenValue, AllocValue, Freeze, FreezeError, Freezer, FrozenHeap,
    FrozenValue, Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Trace, Tracer, Value,
    ValueLike,
};
use starlark_map::small_map::SmallMap;

static RECORD_TYPE_ID: AtomicU64 = AtomicU64::new(0);

fn next_record_type_id() -> u64 {
    RECORD_TYPE_ID.fetch_add(1, Ordering::SeqCst)
}

// -----------------------------------------------------------------------------
// Field
// -----------------------------------------------------------------------------

/// A field definition for a record, containing a type and optional default value.
#[derive(Debug, Clone, ProvidesStaticType, Allocative)]
pub struct Field<'v> {
    pub(crate) typ: TypeCompiled<Value<'v>>,
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
        if let Some(ref mut d) = self.default {
            d.trace(tracer);
        }
    }
}

impl<'v> Field<'v> {
    pub fn freeze(self, freezer: &Freezer) -> Result<FrozenField, FreezeError> {
        Ok(FrozenField {
            typ: self.typ.freeze(freezer)?,
            default: self.default.map(|d| d.freeze(freezer)).transpose()?,
        })
    }
}

/// A frozen field definition.
#[derive(Debug, Clone, ProvidesStaticType, Allocative)]
pub struct FrozenField {
    pub(crate) typ: TypeCompiled<FrozenValue>,
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
            default: self.default.map(|d| d.freeze(freezer)).transpose()?,
        })
    }
}

// -----------------------------------------------------------------------------
// RecordType
// -----------------------------------------------------------------------------

/// The type of a record, created by `record(field1=type1, field2=type2, ...)`.
/// Calling this type creates a `Record` instance.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct RecordType<'v> {
    /// Unique identifier for this record type
    pub(crate) id: u64,
    /// Name of the record type (set when assigned to a variable)
    pub(crate) name: Option<String>,
    /// Fields with their types and optional defaults
    pub(crate) fields: SmallMap<String, Field<'v>>,
}

impl<'v> Display for RecordType<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "record[{}]", name),
            None => write!(f, "record[anon]"),
        }
    }
}

unsafe impl<'v> Trace<'v> for RecordType<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for (_, field) in self.fields.iter_mut() {
            field.trace(tracer);
        }
    }
}

impl<'v> AllocValue<'v> for RecordType<'v> {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "record_type")]
impl<'v> StarlarkValue<'v> for RecordType<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn export_as(
        &self,
        variable_name: &str,
        _eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        // This is called when the record type is assigned to a variable.
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
        // Parse the arguments according to our field definitions
        let mut values: Vec<Cell<Value<'v>>> = Vec::with_capacity(self.fields.len());

        // Get the named arguments
        args.no_positional_args(eval.heap())?;
        let kwargs = args.names_map()?;

        // Build values in field order
        for (field_name, field) in self.fields.iter() {
            let value = if let Some(v) = kwargs.get(field_name.as_str()) {
                *v
            } else if let Some(default) = field.default {
                default
            } else {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Missing required field `{}` for {}",
                    field_name,
                    self
                )));
            };

            // Type check the value
            if !field.typ.matches(value) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Field `{}` expected type `{}`, got `{}`",
                    field_name,
                    field.typ,
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

        let record = Record {
            typ: _me,
            values: values.into_boxed_slice(),
        };
        Ok(eval.heap().alloc(record))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(record_type_methods)
    }
}

#[starlark_module]
fn record_type_methods(_builder: &mut MethodsBuilder) {}

// -----------------------------------------------------------------------------
// FrozenRecordType
// -----------------------------------------------------------------------------

/// Frozen version of RecordType.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenRecordType {
    pub(crate) id: u64,
    pub(crate) name: Option<String>,
    pub(crate) fields: SmallMap<String, FrozenField>,
}

impl Display for FrozenRecordType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "record[{}]", name),
            None => write!(f, "record[anon]"),
        }
    }
}

unsafe impl<'v> Trace<'v> for FrozenRecordType {
    fn trace(&mut self, _tracer: &Tracer<'v>) {
        // Frozen values don't need tracing
    }
}

impl AllocFrozenValue for FrozenRecordType {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "record_type")]
impl<'v> StarlarkValue<'v> for FrozenRecordType {
    type Canonical = RecordType<'v>;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn invoke(
        &self,
        _me: Value<'v>,
        args: &starlark::eval::Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let mut values: Vec<Cell<Value<'v>>> = Vec::with_capacity(self.fields.len());

        args.no_positional_args(eval.heap())?;
        let kwargs = args.names_map()?;

        for (field_name, field) in self.fields.iter() {
            let value = if let Some(v) = kwargs.get(field_name.as_str()) {
                *v
            } else if let Some(default) = field.default {
                default.to_value()
            } else {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Missing required field `{}` for {}",
                    field_name,
                    self
                )));
            };

            // Type check using matches on the value representation
            if !field.typ.matches(value) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Field `{}` expected type `{}`, got `{}`",
                    field_name,
                    field.typ,
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

        let record = Record {
            typ: _me,
            values: values.into_boxed_slice(),
        };
        Ok(eval.heap().alloc(record))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(record_type_methods)
    }
}

impl Freeze for RecordType<'_> {
    type Frozen = FrozenRecordType;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let mut frozen_fields = SmallMap::with_capacity(self.fields.len());
        for (name, field) in self.fields.into_iter() {
            frozen_fields.insert(name, field.freeze(freezer)?);
        }
        Ok(FrozenRecordType {
            id: self.id,
            name: self.name,
            fields: frozen_fields,
        })
    }
}

// -----------------------------------------------------------------------------
// Record
// -----------------------------------------------------------------------------

/// An instance of a record type, containing field values.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Record<'v> {
    /// The record type this instance belongs to
    pub(crate) typ: Value<'v>,
    /// Field values in the same order as the type's field definitions (mutable via Cell)
    #[allocative(skip)]
    pub(crate) values: Box<[Cell<Value<'v>>]>,
}

impl<'v> Display for Record<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.typ)?;
        if let Some(record_type) = self.typ.downcast_ref::<RecordType>() {
            let mut first = true;
            for ((name, _), value) in record_type.fields.iter().zip(self.values.iter()) {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                write!(f, "{}={}", name, value.get())?;
            }
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenRecordType>() {
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

unsafe impl<'v> Trace<'v> for Record<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.typ.trace(tracer);
        for cell in self.values.iter() {
            let mut v = cell.get();
            v.trace(tracer);
            cell.set(v);
        }
    }
}

impl<'v> AllocValue<'v> for Record<'v> {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Record<'v> {
    fn get_field_names(&self) -> Vec<&str> {
        if let Some(record_type) = self.typ.downcast_ref::<RecordType>() {
            record_type.fields.keys().map(|s| s.as_str()).collect()
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenRecordType>() {
            frozen_type.fields.keys().map(|s| s.as_str()).collect()
        } else {
            vec![]
        }
    }
}

#[starlark_value(type = "record")]
impl<'v> StarlarkValue<'v> for Record<'v> {
    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn get_attr(&self, attribute: &str, _heap: &'v Heap) -> Option<Value<'v>> {
        if let Some(record_type) = self.typ.downcast_ref::<RecordType>() {
            if let Some(idx) = record_type.fields.get_index_of(attribute) {
                return Some(self.values[idx].get());
            }
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenRecordType>() {
            if let Some(idx) = frozen_type.fields.get_index_of(attribute) {
                return Some(self.values[idx].get());
            }
        }
        None
    }

    fn set_attr(&self, attribute: &str, value: Value<'v>) -> starlark::Result<()> {
        // Get field info and index
        let (idx, field_typ) = if let Some(record_type) = self.typ.downcast_ref::<RecordType>() {
            if let Some(idx) = record_type.fields.get_index_of(attribute) {
                (idx, &record_type.fields.get_index(idx).unwrap().1.typ)
            } else {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Record {} has no field `{}`",
                    self.typ,
                    attribute
                )));
            }
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenRecordType>() {
            if let Some(idx) = frozen_type.fields.get_index_of(attribute) {
                // For frozen types, we need to check against the frozen field's type
                let field = frozen_type.fields.get_index(idx).unwrap().1;
                if !field.typ.matches(value) {
                    return Err(starlark::Error::new_other(anyhow::anyhow!(
                        "Field `{}` expected type `{}`, got `{}`",
                        attribute,
                        field.typ,
                        value.get_type()
                    )));
                }
                self.values[idx].set(value);
                return Ok(());
            } else {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Record {} has no field `{}`",
                    self.typ,
                    attribute
                )));
            }
        } else {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "Invalid record type"
            )));
        };

        // Type check the value
        if !field_typ.matches(value) {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "Field `{}` expected type `{}`, got `{}`",
                attribute,
                field_typ,
                value.get_type()
            )));
        }

        // Set the value
        self.values[idx].set(value);
        Ok(())
    }

    fn has_attr(&self, attribute: &str, _heap: &'v Heap) -> bool {
        if let Some(record_type) = self.typ.downcast_ref::<RecordType>() {
            record_type.fields.contains_key(attribute)
        } else if let Some(frozen_type) = self.typ.downcast_ref::<FrozenRecordType>() {
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
        if let Some(other_record) = other.downcast_ref::<Record>() {
            // Check that they have the same record type
            let self_id = self
                .typ
                .downcast_ref::<RecordType>()
                .map(|t| t.id)
                .or_else(|| self.typ.downcast_ref::<FrozenRecordType>().map(|t| t.id));
            let other_id = other_record
                .typ
                .downcast_ref::<RecordType>()
                .map(|t| t.id)
                .or_else(|| {
                    other_record
                        .typ
                        .downcast_ref::<FrozenRecordType>()
                        .map(|t| t.id)
                });

            if self_id != other_id {
                return Ok(false);
            }

            // Compare all values
            if self.values.len() != other_record.values.len() {
                return Ok(false);
            }
            for (a, b) in self.values.iter().zip(other_record.values.iter()) {
                if !a.get().equals(b.get())? {
                    return Ok(false);
                }
            }
            Ok(true)
        } else if let Some(other_frozen) = other.downcast_ref::<FrozenRecord>() {
            let self_id = self
                .typ
                .downcast_ref::<RecordType>()
                .map(|t| t.id)
                .or_else(|| self.typ.downcast_ref::<FrozenRecordType>().map(|t| t.id));
            let other_id = other_frozen
                .typ
                .downcast_ref::<FrozenRecordType>()
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
// FrozenRecord
// -----------------------------------------------------------------------------

/// Frozen version of Record.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenRecord {
    pub(crate) typ: FrozenValue,
    pub(crate) values: Box<[FrozenValue]>,
}

impl Display for FrozenRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.typ)?;
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenRecordType>() {
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

unsafe impl<'v> Trace<'v> for FrozenRecord {
    fn trace(&mut self, _tracer: &Tracer<'v>) {
        // Frozen values don't need tracing
    }
}

impl AllocFrozenValue for FrozenRecord {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "record")]
impl<'v> StarlarkValue<'v> for FrozenRecord {
    type Canonical = Record<'v>;

    fn collect_repr(&self, collector: &mut String) {
        write!(collector, "{}", self).unwrap();
    }

    fn get_attr(&self, attribute: &str, _heap: &'v Heap) -> Option<Value<'v>> {
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenRecordType>() {
            if let Some(idx) = frozen_type.fields.get_index_of(attribute) {
                return Some(self.values[idx].to_value());
            }
        }
        None
    }

    fn has_attr(&self, attribute: &str, _heap: &'v Heap) -> bool {
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenRecordType>() {
            frozen_type.fields.contains_key(attribute)
        } else {
            false
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        if let Some(frozen_type) = self.typ.downcast_ref::<FrozenRecordType>() {
            frozen_type.fields.keys().map(|s| s.to_string()).collect()
        } else {
            vec![]
        }
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        if let Some(other_frozen) = other.downcast_ref::<FrozenRecord>() {
            let self_id = self.typ.downcast_ref::<FrozenRecordType>().map(|t| t.id);
            let other_id = other_frozen
                .typ
                .downcast_ref::<FrozenRecordType>()
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
        } else if let Some(other_record) = other.downcast_ref::<Record>() {
            let self_id = self.typ.downcast_ref::<FrozenRecordType>().map(|t| t.id);
            let other_id = other_record
                .typ
                .downcast_ref::<RecordType>()
                .map(|t| t.id)
                .or_else(|| {
                    other_record
                        .typ
                        .downcast_ref::<FrozenRecordType>()
                        .map(|t| t.id)
                });

            if self_id != other_id {
                return Ok(false);
            }

            if self.values.len() != other_record.values.len() {
                return Ok(false);
            }
            for (a, b) in self.values.iter().zip(other_record.values.iter()) {
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

impl Freeze for Record<'_> {
    type Frozen = FrozenRecord;

    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        let typ = self.typ.freeze(freezer)?;
        let values: Result<Vec<_>, _> = self
            .values
            .iter()
            .map(|v| v.get().freeze(freezer))
            .collect();
        Ok(FrozenRecord {
            typ,
            values: values?.into_boxed_slice(),
        })
    }
}

// -----------------------------------------------------------------------------
// Global functions
// -----------------------------------------------------------------------------

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    /// Creates a record type with the given fields.
    ///
    /// Example:
    /// ```starlark
    /// MyRecord = spec(host=str, port=int)
    /// r = MyRecord(host="localhost", port=80)
    /// print(r.host)  # "localhost"
    /// print(r.port)  # 80
    /// ```
    fn spec<'v>(
        #[starlark(kwargs)] kwargs: SmallMap<&str, Value<'v>>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<RecordType<'v>> {
        let mut fields = SmallMap::with_capacity(kwargs.len());

        for (name, value) in kwargs.into_iter() {
            let field = if let Some(field_value) = value.downcast_ref::<FieldValue>() {
                // It's already a field() definition
                Field {
                    typ: field_value.typ.dupe(),
                    default: field_value.default,
                }
            } else {
                // It's a type, convert to a field without default
                let typ = TypeCompiled::new(value, eval.heap())?;
                Field { typ, default: None }
            };
            fields.insert(name.to_string(), field);
        }

        Ok(RecordType {
            id: next_record_type_id(),
            name: None,
            fields,
        })
    }

    /// Creates a field definition with a type and optional default value.
    ///
    /// Example:
    /// ```starlark
    /// MyRecord = spec(host=str, port=attr(int, 80))
    /// r = MyRecord(host="localhost")  # port defaults to 80
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
            default,
        })
    }
}
