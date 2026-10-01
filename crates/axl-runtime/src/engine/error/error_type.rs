//! Error types: the root `error`, the types `.type(...)` derives from it,
//! and the constructor that builds their values.

use std::convert::Infallible;
use std::fmt::{self, Display};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};

use allocative::Allocative;
use dupe::Dupe;
use pagable::Pagable;
use pagable::pagable_typetag;
use starlark::collections::StarlarkHasher;
use starlark::docs::{DocItem, DocMember, DocProperty, DocString, DocStringKind};
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_module;
use starlark::typing::{Ty, TyBasic, TyStarlarkValue, TyUser, TyUserFields, TyUserParams};
use starlark::values::none::NoneOr;
use starlark::values::record::{Field, FrozenField};
use starlark::values::type_repr::StarlarkTypeRepr;
use starlark::values::typing::TypeType;
use starlark::values::typing::{
    TypeCompiled, TypeInstanceId, TypeMatcher, TypeMatcherDyn, TypeMatcherFactory,
};
use starlark::values::{
    AllocFrozenValue, AllocValue, Freeze, FrozenHeap, FrozenValue, Heap, NoSerialize,
    ProvidesStaticType, StarlarkValue, Trace, UnpackValue, Value, ValueLifetimeless, ValueLike,
    starlark_value,
};
use starlark_derive::type_matcher;
use starlark_map::small_map::SmallMap;

use crate::engine::r#trait::{build_type_checkers, copy_default_value};

use super::value::{ERROR_METHODS, ErrorValue, ErrorValueRef};

/// Names every error has, so no error type may declare them as fields.
pub(super) const RESERVED_FIELDS: [&str; 3] = ["message", "stacktrace", "cause"];

static ERROR_TYPE_ID: AtomicU64 = AtomicU64::new(1);

/// The root type's id; every other error type gets a fresh one.
const ROOT_ID: u64 = 0;

/// What an error type is, independent of the heap its field specs live on:
/// shared by the live and frozen copies, so freezing a type is cheap.
#[derive(Debug)]
pub(super) struct ErrorTypeMeta {
    id: u64,
    ty_id: TypeInstanceId,
    /// The ids of the root, every ancestor, and this type itself.
    pub(super) ancestors: Box<[u64]>,
    /// The ancestors as the typechecker sees them, so an annotation of a
    /// parent type accepts a child.
    supertypes: Vec<TyBasic>,
    pub(super) traceback: bool,
    /// What the type's documentation page says about it. A derived type
    /// says what its parent says unless it was given its own.
    docs: &'static str,
}

pub(super) static ROOT_META: LazyLock<Arc<ErrorTypeMeta>> = LazyLock::new(|| {
    Arc::new(ErrorTypeMeta {
        id: ROOT_ID,
        ty_id: TypeInstanceId::r#gen(),
        ancestors: Box::new([ROOT_ID]),
        supertypes: Vec::new(),
        traceback: true,
        docs: ERROR_DOCS,
    })
});

/// Matches the instances of one error type and of all its descendants.
#[derive(Hash, Debug, Eq, PartialEq, Clone, Dupe, Allocative, Pagable)]
#[pagable_typetag(TypeMatcherDyn)]
struct ErrorMatcher {
    id: u64,
}

#[type_matcher]
impl TypeMatcher for ErrorMatcher {
    fn matches(&self, value: Value) -> bool {
        ErrorValueRef::of(value).is_some_and(|e| e.is_instance_of(self.id))
    }
}

/// The `Ty` an error type evaluates to as an annotation.
fn instance_ty(meta: &ErrorTypeMeta, name: &str) -> Ty {
    TyUser::new(
        name.to_owned(),
        TyStarlarkValue::new::<ErrorValue>(),
        meta.ty_id,
        TyUserParams {
            supertypes: meta.supertypes.clone(),
            matcher: Some(TypeMatcherFactory::new(ErrorMatcher { id: meta.id })),
            fields: TyUserFields::unknown(),
            ..TyUserParams::default()
        },
    )
    .map(Ty::custom)
    .unwrap_or_else(|_| Ty::any())
}

/// A declared field: its name, the type its values must match, and the
/// default used when the constructor omits it. `ty` and `docs` are what the
/// type's documentation page shows for it.
#[derive(Debug, Clone, Trace, Freeze, Allocative)]
struct ErrorField<V: ValueLifetimeless> {
    #[trace(static)]
    name: String,
    typ: V,
    default: Option<V>,
    #[trace(static)]
    #[freeze(identity)]
    #[allocative(skip)]
    ty: Ty,
    #[trace(static)]
    #[freeze(identity)]
    #[allocative(skip)]
    docs: Option<&'static str>,
}

/// An error type: the root `error`, or one derived with `.type(...)`.
#[derive(Debug, Trace, Freeze, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ErrorTypeGen<V: ValueLifetimeless> {
    #[trace(static)]
    #[freeze(identity)]
    #[allocative(skip)]
    meta: Arc<ErrorTypeMeta>,
    /// Set by the first assignment that exports the type.
    #[trace(static)]
    #[freeze(identity)]
    #[allocative(skip)]
    name: OnceLock<String>,
    /// Inherited fields first, then this type's own, in declaration order.
    fields: Box<[ErrorField<V>]>,
}

pub type ErrorType<'v> = ErrorTypeGen<Value<'v>>;
pub type FrozenErrorType = ErrorTypeGen<FrozenValue>;

impl<V: ValueLifetimeless> ErrorTypeGen<V> {
    fn name(&self) -> &str {
        self.name
            .get()
            .map_or("anonymous error type", String::as_str)
    }
}

impl<V: ValueLifetimeless> Display for ErrorTypeGen<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl<'v> AllocValue<'v> for ErrorType<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl AllocFrozenValue for FrozenErrorType {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(self)
    }
}

/// The meta of a new type derived from `parent`, named `parent_name`.
fn child_meta(
    parent: &ErrorTypeMeta,
    parent_name: &str,
    traceback: bool,
    docs: &'static str,
) -> ErrorTypeMeta {
    let id = ERROR_TYPE_ID.fetch_add(1, Ordering::SeqCst);
    let mut supertypes = instance_ty(parent, parent_name).iter_union().to_vec();
    supertypes.extend(parent.supertypes.iter().cloned());
    ErrorTypeMeta {
        id,
        ty_id: TypeInstanceId::r#gen(),
        ancestors: parent.ancestors.iter().copied().chain([id]).collect(),
        supertypes,
        traceback,
        docs,
    }
}

/// A field of an error type defined in Rust: its name, its type as a type
/// value and as the typechecker sees it, and what its documentation says.
#[derive(Clone)]
pub(crate) struct NativeField {
    pub(crate) name: &'static str,
    pub(crate) typ: FrozenValue,
    pub(crate) ty: Ty,
    pub(crate) docs: &'static str,
}

/// An error type defined in Rust and derived from `error`, as `native_error!`
/// declares one. It lives in a static, so every copy of the type built from
/// it compares equal and matches the same instances.
pub(crate) struct NativeErrorType {
    meta: Arc<ErrorTypeMeta>,
    name: &'static str,
    fields: Vec<NativeField>,
}

impl NativeErrorType {
    pub(crate) fn new(name: &'static str, docs: &'static str, fields: Vec<NativeField>) -> Self {
        Self {
            meta: Arc::new(child_meta(&ROOT_META, "error", true, docs)),
            name,
            fields,
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        self.name
    }

    /// The type as a value: what the globals register and what its errors'
    /// values carry.
    pub(crate) fn error_type(&self) -> FrozenErrorType {
        let name = OnceLock::new();
        let _ = name.set(self.name.to_owned());
        FrozenErrorType {
            meta: self.meta.dupe(),
            name,
            fields: self
                .fields
                .iter()
                .map(|f| ErrorField {
                    name: f.name.to_owned(),
                    typ: f.typ,
                    default: None,
                    ty: f.ty.clone(),
                    docs: Some(f.docs),
                })
                .collect(),
        }
    }

    /// The type of its instances, as annotations and signatures show it.
    pub(crate) fn instance_ty(&self) -> Ty {
        instance_ty(&self.meta, self.name)
    }

    /// Whether an error of this type is also an instance of `other`.
    pub(super) fn is_a(&self, other: &NativeErrorType) -> bool {
        self.meta.ancestors.contains(&other.meta.id)
    }
}

/// The root `error` type, as registered in the globals.
pub(super) fn root_error_type() -> FrozenErrorType {
    let name = OnceLock::new();
    let _ = name.set("error".to_owned());
    FrozenErrorType {
        meta: ROOT_META.dupe(),
        name,
        fields: Box::new([]),
    }
}

#[starlark_value(type = "error_type")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for ErrorTypeGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    type Canonical = FrozenErrorType;

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(self.name());
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        std::hash::Hasher::write_u64(hasher, self.meta.id);
        Ok(())
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(ErrorTypeRef::of(other).is_some_and(|t| t.meta().id == self.meta.id))
    }

    fn eval_type(&self) -> Option<Ty> {
        Some(instance_ty(&self.meta, self.name()))
    }

    fn export_as(
        &self,
        variable_name: &str,
        _eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        // The first name sticks, so `Alias = MyError` keeps reporting `MyError`.
        let _ = self.name.set(variable_name.to_owned());
        Ok(())
    }

    fn invoke(
        &self,
        me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let ty = ErrorTypeRef::of(me).expect("an error type invoked as itself");
        construct(me, ty, args, eval)
    }

    fn get_methods() -> Option<&'static Methods> {
        Some(ERROR_TYPE_METHODS.methods())
    }

    // One page per error type: what it is, deriving types, the attributes
    // every error value has, and the fields this type's values add.
    fn documentation(&self) -> DocItem {
        let ty = Self::get_type_starlark_repr();
        let mut doc = ERROR_TYPE_METHODS.methods().documentation(ty.clone());
        let values = ERROR_METHODS.methods().documentation(ty);
        doc.members.extend(values.members);
        for field in &self.fields {
            doc.members.insert(
                field.name.clone(),
                DocMember::Property(DocProperty {
                    docs: field
                        .docs
                        .and_then(|d| DocString::from_docstring(DocStringKind::Rust, d)),
                    typ: field.ty.clone(),
                }),
            );
        }
        doc.docs = DocString::from_docstring(DocStringKind::Rust, self.meta.docs);
        DocItem::Type(doc)
    }
}

static ERROR_TYPE_METHODS: MethodsStatic =
    MethodsStatic::new("error_type_methods", error_type_methods);

const ERROR_DOCS: &str = r#"The root error type, and the way to make your own.

An error is a value. `error("message")` builds one; `.type(...)` on `error`,
or on any type derived from it, derives a new error type whose constructor
takes the message, then `cause =` and its fields by name.

Every error has `message`, `cause` and `stacktrace`, documented below, plus the
fields of its type. `type(e) == "error"` for every error, and
`isinstance(e, T)` holds when `e` was built from `T` or from a type derived
from it. `str(e)` is the message.

`fail(e)` raises `e` itself. `future.catch()` turns a failure into an
`(err, value)` pair instead of raising.

```starlark
DeployError = error.type(fields = {"deployment": str})
NotLoggedIn = error.type(traceback = False)

err, resp = ctx.http().get(url = url).catch().block()
if err:
    fail(DeployError("upload failed", deployment = "prod", cause = err))
```
"#;

/// A live or frozen error type, read without caring which.
#[derive(Clone, Copy)]
pub(super) enum ErrorTypeRef<'v> {
    Live(&'v ErrorType<'v>),
    Frozen(&'v FrozenErrorType),
}

impl<'v> ErrorTypeRef<'v> {
    pub(super) fn of(value: Value<'v>) -> Option<Self> {
        if let Some(t) = value.downcast_ref::<ErrorType<'v>>() {
            Some(Self::Live(t))
        } else {
            value.downcast_ref::<FrozenErrorType>().map(Self::Frozen)
        }
    }

    pub(super) fn meta(self) -> &'v Arc<ErrorTypeMeta> {
        match self {
            Self::Live(t) => &t.meta,
            Self::Frozen(t) => &t.meta,
        }
    }

    pub(super) fn name(self) -> &'v str {
        match self {
            Self::Live(t) => t.name(),
            Self::Frozen(t) => t.name(),
        }
    }

    pub(super) fn field_count(self) -> usize {
        match self {
            Self::Live(t) => t.fields.len(),
            Self::Frozen(t) => t.fields.len(),
        }
    }

    /// The `i`th field: its name, type value, and default.
    pub(super) fn field(self, i: usize) -> (&'v str, Value<'v>, Option<Value<'v>>) {
        match self {
            Self::Live(t) => {
                let f = &t.fields[i];
                (f.name.as_str(), f.typ, f.default)
            }
            Self::Frozen(t) => {
                let f = &t.fields[i];
                (
                    f.name.as_str(),
                    f.typ.to_value(),
                    f.default.map(FrozenValue::to_value),
                )
            }
        }
    }

    /// The `i`th field, as a type derived from this one inherits it.
    fn inherit(self, i: usize) -> ErrorField<Value<'v>> {
        let (name, typ, default) = self.field(i);
        let (ty, docs) = match self {
            Self::Live(t) => (&t.fields[i].ty, t.fields[i].docs),
            Self::Frozen(t) => (&t.fields[i].ty, t.fields[i].docs),
        };
        ErrorField {
            name: name.to_owned(),
            typ,
            default,
            ty: ty.clone(),
            docs,
        }
    }

    pub(super) fn field_index(self, name: &str) -> Option<usize> {
        (0..self.field_count()).find(|&i| self.field(i).0 == name)
    }
}

fn error_type_err(msg: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!(msg))
}

/// Build an instance of `ty` from a constructor call.
fn construct<'v>(
    me: Value<'v>,
    ty: ErrorTypeRef<'v>,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let heap = eval.heap();
    let name = ty.name();

    let positional: Vec<Value<'v>> = args.positions(heap)?.collect();
    let kwargs = args.names_map()?;
    if positional.len() > 1 {
        return Err(error_type_err(format!(
            "{name}() takes the message as its only positional argument; pass fields by name"
        )));
    }
    let message = match (positional.first(), kwargs.get("message")) {
        (Some(_), Some(_)) => {
            return Err(error_type_err(format!("{name}() got the message twice")));
        }
        (Some(v), None) | (None, Some(v)) => {
            v.unpack_str().map(str::to_owned).ok_or_else(|| {
                error_type_err(format!(
                    "{name}() message must be a string, got `{}`",
                    v.get_type()
                ))
            })?
        }
        (None, None) => {
            return Err(error_type_err(format!("{name}() is missing its message")));
        }
    };

    let cause = kwargs.get("cause").copied().unwrap_or_else(Value::new_none);
    if !cause.is_none() && ErrorValueRef::of(cause).is_none() {
        return Err(error_type_err(format!(
            "{name}() cause must be an error or None, got `{}`",
            cause.get_type()
        )));
    }

    let count = ty.field_count();
    let checkers = build_type_checkers((0..count).map(|i| ty.field(i).1), heap)?;
    let mut values = Vec::with_capacity(count);
    for (i, checker) in checkers.iter().enumerate() {
        let (field, _, default) = ty.field(i);
        let value = match (kwargs.get(field), default) {
            (Some(v), _) => *v,
            (None, Some(d)) => copy_default_value(d, heap).map_err(starlark::Error::new_other)?,
            (None, None) => {
                return Err(error_type_err(format!(
                    "{name}() is missing required field `{field}`"
                )));
            }
        };
        if !checker.matches(value) {
            return Err(error_type_err(format!(
                "{name}() field `{field}` expected type `{checker}`, got `{}`",
                value.get_type()
            )));
        }
        values.push(value);
    }

    for (key, _) in kwargs.iter() {
        let key = key.as_str();
        if key != "message" && key != "cause" && ty.field_index(key).is_none() {
            return Err(error_type_err(format!(
                "{name}() got an unexpected field `{key}`"
            )));
        }
    }

    Ok(heap.alloc_complex(ErrorValue {
        typ: me,
        message,
        cause,
        values: values.into_boxed_slice(),
        stack: eval.call_stack(),
    }))
}

/// A field spec from `.type(fields = ...)`: a bare type, or a `field()`,
/// exactly as `record()` takes them.
fn field_spec<'v>(
    name: &str,
    spec: Value<'v>,
    heap: Heap<'v>,
) -> anyhow::Result<ErrorField<Value<'v>>> {
    let (typ, default, ty) = if let Some(field) = Field::from_value(spec) {
        // A compiled type is a type value in its own right.
        let ty = field.typ().as_ty().clone();
        (heap.alloc(field.typ().dupe()), field.default().copied(), ty)
    } else {
        let compiled =
            TypeCompiled::new(spec, heap).map_err(|e| anyhow::anyhow!("field `{name}`: {e}"))?;
        (spec, None, compiled.as_ty().clone())
    };
    Ok(ErrorField {
        name: name.to_owned(),
        typ,
        default,
        ty,
        docs: None,
    })
}

/// A field spec as `.type(fields = ...)` takes it: a type, or a `field()`.
/// Unpacking takes anything; `field_spec` says what is wrong with it.
struct FieldSpec<'v>(Value<'v>);

impl StarlarkTypeRepr for FieldSpec<'_> {
    type Canonical = Self;

    fn starlark_type_repr() -> Ty {
        Ty::union2(
            TypeType::starlark_type_repr(),
            Ty::starlark_value::<FrozenField>(),
        )
    }
}

impl<'v> UnpackValue<'v> for FieldSpec<'v> {
    type Error = Infallible;

    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Infallible> {
        Ok(Some(Self(value)))
    }
}

#[starlark_module]
fn error_type_methods(builder: &mut MethodsBuilder) {
    /// Derive a new error type from this one.
    ///
    /// The new type inherits every field of this type and adds `fields`.
    /// Its instances are also instances of this type and of each of its
    /// ancestors, for both `isinstance` and type annotations. The name comes
    /// from the variable the type is first assigned to, so declare error types
    /// at module top level.
    ///
    /// `fields` maps each field name to a type, or to `field(type, default = ...)`
    /// for an optional field, as `record()` does. `message`, `stacktrace` and
    /// `cause` are reserved, and an inherited field cannot be redeclared.
    ///
    /// `traceback` decides how an error of this type renders when it escapes
    /// the task. `True` shows the traceback, as for a bug. `False` prints the
    /// message alone as an `ERROR:` line and exits with code 1, as
    /// `ctx.std.process.exit` does: use it for expected refusals. Unset, it is
    /// inherited; the root `error` has `True`.
    ///
    /// ```starlark
    /// DeployError = error.type(fields = {"deployment": str, "attempt": field(int, default = 1)})
    /// DeployTimeout = DeployError.type(fields = {"after_ms": int})
    /// NotLoggedIn = error.type(traceback = False)
    /// ```
    fn r#type<'v>(
        this: Value<'v>,
        #[starlark(require = named)] fields: Option<SmallMap<String, FieldSpec<'v>>>,
        #[starlark(require = named, default = NoneOr::None)] traceback: NoneOr<bool>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<ErrorType<'v>> {
        let parent = ErrorTypeRef::of(this).expect("`type` is only bound on error types");
        let heap = eval.heap();
        let mut all: Vec<ErrorField<Value<'v>>> = (0..parent.field_count())
            .map(|i| parent.inherit(i))
            .collect();
        for (name, FieldSpec(spec)) in fields.unwrap_or_default() {
            if RESERVED_FIELDS.contains(&name.as_str()) {
                anyhow::bail!("`{name}` is reserved: every error already has it");
            }
            if all.iter().any(|f| f.name == name) {
                anyhow::bail!("field `{name}` is already declared by {}", parent.name());
            }
            all.push(field_spec(&name, spec, heap)?);
        }

        let parent_meta = parent.meta();
        let traceback = traceback.into_option().unwrap_or(parent_meta.traceback);
        Ok(ErrorType {
            meta: Arc::new(child_meta(
                parent_meta,
                parent.name(),
                traceback,
                parent_meta.docs,
            )),
            name: OnceLock::new(),
            fields: all.into_boxed_slice(),
        })
    }
}

/// The type every error value has, as annotations and signatures show it.
pub(crate) fn error_instance_ty() -> Ty {
    instance_ty(&ROOT_META, "error")
}

/// The id of the error type `value`, if it is one.
pub(crate) fn error_type_id(value: Value) -> Option<u64> {
    ErrorTypeRef::of(value).map(|t| t.meta().id)
}
