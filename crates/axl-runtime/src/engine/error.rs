//! First-class error values: the `error` global, the types derived from it,
//! and the payload that carries a raised error through the evaluator.
//!
//! `error` is itself an error type. Calling any error type builds an error
//! value; `.type(...)` on any error type derives a child type. Every value
//! reports `type(e) == "error"`, and `isinstance(e, T)` holds for `T` and each
//! of its ancestors: an instance's type carries the ids of its whole chain, so
//! matching is a scan of that short list.
//!
//! `fail(e)` raises a value as a [`RaisedError`], the `'static` payload an
//! `anyhow::Error` can hold, while the value itself stays live on the heap.
//! Whoever catches it (today, `future.catch()`) gets back that same value. A type declared with
//! `traceback = False` makes its raised errors end the task the way
//! `ctx.std.process.exit` does: [`RaisedError::exit`] is what the `TaskExit`
//! consumers look for.

use std::cell::RefCell;
use std::fmt::{self, Display, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};

use allocative::Allocative;
use dupe::Dupe;
use pagable::Pagable;
use pagable::pagable_typetag;
use starlark::collections::StarlarkHasher;
use starlark::docs::{DocItem, DocString, DocStringKind};
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::{Arguments, CallStack, Evaluator};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::typing::{Ty, TyBasic, TyStarlarkValue, TyUser, TyUserFields, TyUserParams};
use starlark::values::list::AllocList;
use starlark::values::none::NoneOr;
use starlark::values::tuple::UnpackTuple;
use starlark::values::typing::{
    StarlarkNever, TypeCompiled, TypeInstanceId, TypeMatcher, TypeMatcherDyn, TypeMatcherFactory,
};
use starlark::values::{
    AllocFrozenValue, AllocValue, Freeze, FreezeResult, Freezer, FrozenHeap, FrozenValue, Heap,
    NoSerialize, ProvidesStaticType, StarlarkValue, Trace, Value, ValueLifetimeless, ValueLike,
    starlark_value,
};
use starlark_derive::type_matcher;
use starlark_map::small_map::SmallMap;

use super::r#trait::{
    ConfigAttrValue, FrozenConfigAttrValue, build_type_checkers, copy_default_value,
};
use crate::eval::{EvalError, TaskExit};

/// Names every error has, so no error type may declare them as fields.
const RESERVED_FIELDS: [&str; 3] = ["message", "stacktrace", "cause"];

static ERROR_TYPE_ID: AtomicU64 = AtomicU64::new(1);

/// The root type's id; every other error type gets a fresh one.
const ROOT_ID: u64 = 0;

/// What an error type is, independent of the heap its field specs live on:
/// shared by the live and frozen copies, so freezing a type is cheap.
#[derive(Debug)]
struct ErrorTypeMeta {
    id: u64,
    ty_id: TypeInstanceId,
    /// The ids of the root, every ancestor, and this type itself.
    ancestors: Box<[u64]>,
    /// The ancestors as the typechecker sees them, so an annotation of a
    /// parent type accepts a child.
    supertypes: Vec<TyBasic>,
    traceback: bool,
}

static ROOT_META: LazyLock<Arc<ErrorTypeMeta>> = LazyLock::new(|| {
    Arc::new(ErrorTypeMeta {
        id: ROOT_ID,
        ty_id: TypeInstanceId::r#gen(),
        ancestors: Box::new([ROOT_ID]),
        supertypes: Vec::new(),
        traceback: true,
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
/// default used when the constructor omits it.
#[derive(Debug, Clone, Trace, Freeze, Allocative)]
struct ErrorField<V: ValueLifetimeless> {
    #[trace(static)]
    name: String,
    typ: V,
    default: Option<V>,
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

/// The root `error` type, as registered in the globals.
fn root_error_type() -> FrozenErrorType {
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

    // One page for `error`: what errors are, deriving types, and the
    // attributes every error value has.
    fn documentation(&self) -> DocItem {
        let ty = Self::get_type_starlark_repr();
        let mut doc = ERROR_TYPE_METHODS.methods().documentation(ty.clone());
        let values = ERROR_METHODS.methods().documentation(ty);
        doc.members.extend(values.members);
        doc.docs = DocString::from_docstring(DocStringKind::Rust, ERROR_DOCS);
        DocItem::Type(doc)
    }
}

static ERROR_TYPE_METHODS: MethodsStatic =
    MethodsStatic::new("error_type_methods", error_type_methods);
static ERROR_METHODS: MethodsStatic = MethodsStatic::new("error_methods", error_methods);

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
enum ErrorTypeRef<'v> {
    Live(&'v ErrorType<'v>),
    Frozen(&'v FrozenErrorType),
}

impl<'v> ErrorTypeRef<'v> {
    fn of(value: Value<'v>) -> Option<Self> {
        if let Some(t) = value.downcast_ref::<ErrorType<'v>>() {
            Some(Self::Live(t))
        } else {
            value.downcast_ref::<FrozenErrorType>().map(Self::Frozen)
        }
    }

    fn meta(self) -> &'v Arc<ErrorTypeMeta> {
        match self {
            Self::Live(t) => &t.meta,
            Self::Frozen(t) => &t.meta,
        }
    }

    fn name(self) -> &'v str {
        match self {
            Self::Live(t) => t.name(),
            Self::Frozen(t) => t.name(),
        }
    }

    fn field_count(self) -> usize {
        match self {
            Self::Live(t) => t.fields.len(),
            Self::Frozen(t) => t.fields.len(),
        }
    }

    /// The `i`th field: its name, type value, and default.
    fn field(self, i: usize) -> (&'v str, Value<'v>, Option<Value<'v>>) {
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

    fn field_index(self, name: &str) -> Option<usize> {
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

/// A field spec from `.type(fields = ...)`: a bare type, or an `attr()`,
/// exactly as `trait()` takes them.
fn field_spec<'v>(
    name: &str,
    spec: Value<'v>,
    heap: Heap<'v>,
) -> anyhow::Result<ErrorField<Value<'v>>> {
    let (typ, default) = if let Some(attr) = spec.downcast_ref::<ConfigAttrValue>() {
        (attr.typ_value, attr.default)
    } else if let Some(attr) = spec.downcast_ref::<FrozenConfigAttrValue>() {
        (
            attr.typ_value.to_value(),
            attr.default.map(FrozenValue::to_value),
        )
    } else {
        TypeCompiled::new(spec, heap).map_err(|e| anyhow::anyhow!("field `{name}`: {e}"))?;
        (spec, None)
    };
    Ok(ErrorField {
        name: name.to_owned(),
        typ,
        default,
    })
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
    /// `fields` maps each field name to a type, or to `attr(type, default = ...)`
    /// for an optional field, as `trait()` does. `message`, `stacktrace` and
    /// `cause` are reserved, and an inherited field cannot be redeclared.
    ///
    /// `traceback` decides how an error of this type renders when it escapes
    /// the task. `True` shows the traceback, as for a bug. `False` prints the
    /// message alone as an `ERROR:` line and exits with code 1, as
    /// `ctx.std.process.exit` does: use it for expected refusals. Unset, it is
    /// inherited; the root `error` has `True`.
    ///
    /// ```starlark
    /// DeployError = error.type(fields = {"deployment": str, "attempt": attr(int, default = 1)})
    /// DeployTimeout = DeployError.type(fields = {"after_ms": int})
    /// NotLoggedIn = error.type(traceback = False)
    /// ```
    fn r#type<'v>(
        this: Value<'v>,
        #[starlark(require = named)] fields: Option<SmallMap<String, Value<'v>>>,
        #[starlark(require = named, default = NoneOr::None)] traceback: NoneOr<bool>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<ErrorType<'v>> {
        let parent = ErrorTypeRef::of(this).expect("`type` is only bound on error types");
        let heap = eval.heap();
        let mut all: Vec<ErrorField<Value<'v>>> = (0..parent.field_count())
            .map(|i| {
                let (name, typ, default) = parent.field(i);
                ErrorField {
                    name: name.to_owned(),
                    typ,
                    default,
                }
            })
            .collect();
        for (name, spec) in fields.unwrap_or_default() {
            if RESERVED_FIELDS.contains(&name.as_str()) {
                anyhow::bail!("`{name}` is reserved: every error already has it");
            }
            if all.iter().any(|f| f.name == name) {
                anyhow::bail!("field `{name}` is already declared by {}", parent.name());
            }
            all.push(field_spec(&name, spec, heap)?);
        }

        let parent_meta = parent.meta();
        let id = ERROR_TYPE_ID.fetch_add(1, Ordering::SeqCst);
        let mut supertypes = instance_ty(parent_meta, parent.name())
            .iter_union()
            .to_vec();
        supertypes.extend(parent_meta.supertypes.iter().cloned());
        let meta = ErrorTypeMeta {
            id,
            ty_id: TypeInstanceId::r#gen(),
            ancestors: parent_meta.ancestors.iter().copied().chain([id]).collect(),
            supertypes,
            traceback: traceback.into_option().unwrap_or(parent_meta.traceback),
        };
        Ok(ErrorType {
            meta: Arc::new(meta),
            name: OnceLock::new(),
            fields: all.into_boxed_slice(),
        })
    }
}

/// An error value. `typ` is the error type it was built from, or `None` for a
/// plain `error` the runtime built from a Rust error.
#[derive(Debug, Trace, Freeze, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ErrorValueGen<V: ValueLifetimeless> {
    typ: V,
    #[trace(static)]
    message: String,
    cause: V,
    values: Box<[V]>,
    #[trace(static)]
    #[freeze(identity)]
    #[allocative(skip)]
    stack: CallStack,
}

pub type ErrorValue<'v> = ErrorValueGen<Value<'v>>;
pub type FrozenErrorValue = ErrorValueGen<FrozenValue>;

impl<'v, V: ValueLike<'v>> Display for ErrorValueGen<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// A live or frozen error value, read without caring which.
#[derive(Clone, Copy)]
pub(crate) enum ErrorValueRef<'v> {
    Live(&'v ErrorValue<'v>),
    Frozen(&'v FrozenErrorValue),
}

impl<'v> ErrorValueRef<'v> {
    pub(crate) fn of(value: Value<'v>) -> Option<Self> {
        if let Some(e) = value.downcast_ref::<ErrorValue<'v>>() {
            Some(Self::Live(e))
        } else {
            value.downcast_ref::<FrozenErrorValue>().map(Self::Frozen)
        }
    }

    fn typ(self) -> Option<ErrorTypeRef<'v>> {
        let typ = match self {
            Self::Live(e) => e.typ,
            Self::Frozen(e) => e.typ.to_value(),
        };
        ErrorTypeRef::of(typ)
    }

    fn meta(self) -> &'v ErrorTypeMeta {
        self.typ().map_or(&ROOT_META, |t| t.meta())
    }

    fn type_name(self) -> &'v str {
        self.typ().map_or("error", |t| t.name())
    }

    fn message(self) -> &'v str {
        match self {
            Self::Live(e) => &e.message,
            Self::Frozen(e) => &e.message,
        }
    }

    fn cause(self) -> Value<'v> {
        match self {
            Self::Live(e) => e.cause,
            Self::Frozen(e) => e.cause.to_value(),
        }
    }

    fn stack(self) -> &'v CallStack {
        match self {
            Self::Live(e) => &e.stack,
            Self::Frozen(e) => &e.stack,
        }
    }

    /// Whether this error is an instance of the error type with id `type_id`.
    pub(crate) fn is_instance_of(self, type_id: u64) -> bool {
        self.meta().ancestors.contains(&type_id)
    }

    fn traceback(self) -> bool {
        self.meta().traceback
    }
}

fn this_error<'v>(this: Value<'v>) -> ErrorValueRef<'v> {
    ErrorValueRef::of(this).expect("error attribute bound on a non-error value")
}

#[starlark_value(type = "error")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for ErrorValueGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    type Canonical = FrozenErrorValue;

    fn collect_repr(&self, collector: &mut String) {
        let name = ErrorTypeRef::of(self.typ.to_value()).map_or("error", |t| t.name());
        write!(collector, "{name}(message = {:?}", self.message).unwrap();
        if let Some(t) = ErrorTypeRef::of(self.typ.to_value()) {
            for (i, v) in self.values.iter().enumerate() {
                write!(collector, ", {} = ", t.field(i).0).unwrap();
                v.to_value().collect_repr(collector);
            }
        }
        if !self.cause.to_value().is_none() {
            collector.push_str(", cause = ");
            self.cause.to_value().collect_repr(collector);
        }
        collector.push(')');
    }

    fn collect_str(&self, collector: &mut String) {
        collector.push_str(&self.message);
    }

    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        let t = ErrorTypeRef::of(self.typ.to_value())?;
        t.field_index(attribute).map(|i| self.values[i].to_value())
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        ErrorTypeRef::of(self.typ.to_value()).is_some_and(|t| t.field_index(attribute).is_some())
    }

    // `message`, `cause` and `stacktrace` are attributes, listed already.
    fn dir_attr(&self) -> Vec<String> {
        ErrorTypeRef::of(self.typ.to_value()).map_or_else(Vec::new, |t| {
            (0..t.field_count())
                .map(|i| t.field(i).0.to_owned())
                .collect()
        })
    }

    fn get_methods() -> Option<&'static Methods> {
        Some(ERROR_METHODS.methods())
    }
}

#[starlark_module]
fn error_methods(builder: &mut MethodsBuilder) {
    /// What went wrong, as given to the constructor. `str(e)` returns it too.
    #[starlark(attribute)]
    fn message<'v>(this: Value<'v>) -> starlark::Result<String> {
        Ok(this_error(this).message().to_owned())
    }

    /// The error that led to this one, or `None`.
    #[starlark(attribute)]
    fn cause<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(this_error(this).cause())
    }

    /// Where the error was constructed, as a list of frames, outermost call
    /// first. For an error the runtime built from a failed operation, it is
    /// where that failure reached AXL, e.g. the `future.block()` call.
    #[starlark(attribute)]
    fn stacktrace<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        // A frame records where its function was *called*, so, like
        // Starlark's traceback, pair each location with the caller's name.
        // The innermost frame is the error's own constructor, and drops out.
        let mut caller = "<module>";
        let mut frames = Vec::new();
        for frame in &this_error(this).stack().frames {
            frames.push(ErrorFrame::new(caller, frame.location.as_ref()));
            caller = &frame.name;
        }
        Ok(heap.alloc(AllocList(frames)))
    }
}

/// One frame of an error's `stacktrace`.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ErrorFrame {
    name: String,
    /// File, 1-based line and 1-based column of the call, when known.
    location: Option<(String, u32, u32)>,
}

impl ErrorFrame {
    fn new(name: &str, location: Option<&starlark::codemap::FileSpan>) -> Self {
        let location = location.map(|span| {
            let begin = span.resolve_span().begin;
            (
                span.filename().to_owned(),
                begin.line as u32 + 1,
                begin.column as u32 + 1,
            )
        });
        Self {
            name: name.to_owned(),
            location,
        }
    }
}

impl Display for ErrorFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.location {
            Some((path, line, column)) => write!(f, "{}:{line}:{column}: in {}", path, self.name),
            None => write!(f, "<native>: in {}", self.name),
        }
    }
}

starlark_simple_value!(ErrorFrame);

#[starlark_value(type = "error_frame")]
impl<'v> StarlarkValue<'v> for ErrorFrame {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("error_frame_methods", error_frame_methods);
        Some(RES.methods())
    }
}

fn this_frame<'v>(this: Value<'v>) -> &'v ErrorFrame {
    this.downcast_ref::<ErrorFrame>()
        .expect("frame attribute bound on a non-frame value")
}

#[starlark_module]
fn error_frame_methods(builder: &mut MethodsBuilder) {
    /// The function running in this frame.
    #[starlark(attribute)]
    fn name<'v>(this: Value<'v>) -> starlark::Result<String> {
        Ok(this_frame(this).name.clone())
    }

    /// The file of the call, or `None` for a native frame.
    #[starlark(attribute)]
    fn path<'v>(this: Value<'v>) -> starlark::Result<NoneOr<String>> {
        Ok(NoneOr::from_option(
            this_frame(this).location.as_ref().map(|l| l.0.clone()),
        ))
    }

    /// The 1-based line of the call, or `None` for a native frame.
    #[starlark(attribute)]
    fn line<'v>(this: Value<'v>) -> starlark::Result<NoneOr<u32>> {
        Ok(NoneOr::from_option(
            this_frame(this).location.as_ref().map(|l| l.1),
        ))
    }

    /// The 1-based column of the call, or `None` for a native frame.
    #[starlark(attribute)]
    fn column<'v>(this: Value<'v>) -> starlark::Result<NoneOr<u32>> {
        Ok(NoneOr::from_option(
            this_frame(this).location.as_ref().map(|l| l.2),
        ))
    }
}

/// The raised error values of one module, kept live on its heap.
///
/// A raised value cannot ride inside an `anyhow::Error`, which must be
/// `'static`, and freezing it would forward the live value (and every value
/// its fields reach) out from under the code still holding it. So `fail(e)`
/// parks `e` here, in the module's traced `extra_value` slot where the
/// garbage collector sees it, and the error carries only the slot's index.
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
struct RaisedValues<'v> {
    #[trace(static)]
    id: u64,
    #[allocative(skip)]
    values: RefCell<Vec<Value<'v>>>,
}

static RAISED_VALUES_ID: AtomicU64 = AtomicU64::new(0);

impl Display for RaisedValues<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("raised_values")
    }
}

#[starlark_value(type = "raised_values")]
impl<'v> StarlarkValue<'v> for RaisedValues<'v> {}

/// A module that finishes loading keeps none of its raised values.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, derive_more::Display)]
#[display("raised_values")]
struct FrozenRaisedValues;

starlark_simple_value!(FrozenRaisedValues);

#[starlark_value(type = "raised_values")]
impl<'v> StarlarkValue<'v> for FrozenRaisedValues {
    type Canonical = RaisedValues<'v>;
}

impl Freeze for RaisedValues<'_> {
    type Frozen = FrozenRaisedValues;

    fn freeze(self, _freezer: &Freezer) -> FreezeResult<Self::Frozen> {
        Ok(FrozenRaisedValues)
    }
}

impl<'v> RaisedValues<'v> {
    /// The current module's store, created on first use. `None` if the slot
    /// already holds something else.
    fn of(eval: &Evaluator<'v, '_, '_>) -> Option<&'v RaisedValues<'v>> {
        let module = eval.module();
        let slot = match module.extra_value() {
            Some(v) => v,
            None => {
                let v = eval.heap().alloc_complex(RaisedValues {
                    id: RAISED_VALUES_ID.fetch_add(1, Ordering::SeqCst),
                    values: RefCell::new(Vec::new()),
                });
                module.set_extra_value(v);
                v
            }
        };
        slot.downcast_ref::<RaisedValues<'v>>()
    }
}

/// A raised error value: what `fail(e)` produces.
///
/// It renders as `TypeName: message`, the line the traceback ends on. For a
/// type declared with `traceback = False`, [`RaisedError::exit`] is the
/// `TaskExit` the runtime reports instead, so the error ends the task as
/// `ctx.std.process.exit(1, message)` would. Whoever catches it gets the value
/// back with [`RaisedError::value`].
///
/// Like `TaskExit`, it is only found at the root of an `anyhow` chain, or
/// inside the `EvalError` that wraps it there.
#[derive(Debug)]
pub struct RaisedError {
    /// The store's id and the value's index in it, when it could be parked.
    slot: Option<(u64, usize)>,
    rendered: String,
    exit: Option<TaskExit>,
}

impl RaisedError {
    fn new<'v>(value: Value<'v>, error: ErrorValueRef<'v>, eval: &Evaluator<'v, '_, '_>) -> Self {
        let message = error.message();
        let (rendered, exit) = if error.traceback() {
            (format!("{}: {message}", error.type_name()), None)
        } else {
            (message.to_owned(), Some(TaskExit::error(message)))
        };
        let slot = RaisedValues::of(eval).map(|store| {
            let mut values = store.values.borrow_mut();
            values.push(value);
            (store.id, values.len() - 1)
        });
        Self {
            slot,
            rendered,
            exit,
        }
    }

    /// The exit the raise stands for, if its type has `traceback = False`.
    pub fn exit(&self) -> Option<&TaskExit> {
        self.exit.as_ref()
    }

    /// The raised value, if it was raised in the module `eval` is running.
    fn value<'v>(&self, eval: &Evaluator<'v, '_, '_>) -> Option<Value<'v>> {
        let (id, index) = self.slot?;
        let store = eval
            .module()
            .extra_value()?
            .downcast_ref::<RaisedValues<'v>>()?;
        if store.id != id {
            return None;
        }
        store.values.borrow().get(index).copied()
    }

    /// The raised error carried by `err`, however the evaluator wrapped it.
    pub fn from_starlark(err: &starlark::Error) -> Option<&RaisedError> {
        match err.kind() {
            starlark::ErrorKind::Native(e) | starlark::ErrorKind::Other(e) => Self::from_anyhow(e),
            _ => None,
        }
    }

    /// The raised error carried by `err`, following `EvalError` links
    /// however deeply they nest.
    pub fn from_anyhow(err: &anyhow::Error) -> Option<&RaisedError> {
        if let Some(raised) = err.downcast_ref::<RaisedError>() {
            return Some(raised);
        }
        match err.downcast_ref::<EvalError>()? {
            EvalError::StarlarkError(e) => Self::from_starlark(e),
            EvalError::UnknownError(e) => Self::from_anyhow(e),
            _ => None,
        }
    }
}

impl Display for RaisedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.rendered)
    }
}

impl std::error::Error for RaisedError {}

/// The id of the error type `value`, if it is one.
pub(crate) fn error_type_id(value: Value) -> Option<u64> {
    ErrorTypeRef::of(value).map(|t| t.meta().id)
}

/// `err` as an error value on the evaluator's heap: the value itself when it
/// was raised with `fail(e)`, or else a plain `error` whose message is the top
/// of the chain and whose `cause` holds the rest, one link per `source()`.
pub(crate) fn error_value_of<'v>(
    err: &anyhow::Error,
    eval: &mut Evaluator<'v, '_, '_>,
) -> Value<'v> {
    if let Some(value) = RaisedError::from_anyhow(err).and_then(|r| r.value(eval)) {
        return value;
    }
    let heap = eval.heap();
    let stack = eval.call_stack();
    let links: Vec<String> = err.chain().map(ToString::to_string).collect();
    links
        .into_iter()
        .rev()
        .fold(Value::new_none(), |cause, message| {
            heap.alloc_complex(ErrorValue {
                typ: Value::new_none(),
                message,
                cause,
                values: Box::new([]),
                stack: stack.clone(),
            })
        })
}

#[starlark_module]
fn register_fail(globals: &mut GlobalsBuilder) {
    /// Fail the execution.
    ///
    /// With a single error value, raises that error itself: its type,
    /// fields, `cause` and `stacktrace` survive, so a caller that catches it
    /// gets back the same value, and one that does not renders it according
    /// to its type's `traceback` setting. Re-raise a caught error the same
    /// way.
    ///
    /// With anything else, behaves as Starlark's `fail`: the arguments are
    /// joined with spaces (strings as-is, other values as their repr) into
    /// the message of a failure that shows its traceback.
    ///
    /// ```starlark
    /// fail(DeployError("no ack", deployment = "prod"))
    /// fail("unexpected state:", state)
    /// ```
    fn fail<'v>(
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkNever> {
        if let [value] = args.items[..] {
            if let Some(error) = ErrorValueRef::of(value) {
                let raised = RaisedError::new(value, error, eval);
                return Err(starlark::Error::new_native(anyhow::Error::new(raised)));
            }
        }
        // Starlark's own `fail`, verbatim, so string failures read as before.
        let mut s = String::new();
        for x in args.items {
            s.push(' ');
            match x.unpack_str() {
                Some(x) => s.push_str(x),
                None => x.collect_repr(&mut s),
            }
        }
        Err(starlark::Error::new_kind(starlark::ErrorKind::Fail(
            anyhow::Error::msg(s),
        )))
    }
}

/// Register `error` and the `fail` that raises error values. Call after the
/// Starlark standard library, so this `fail` replaces the builtin one.
pub fn register_globals(globals: &mut GlobalsBuilder) {
    globals.set("error", root_error_type());
    register_fail(globals);
    #[cfg(test)]
    test_future::register(globals);
}

/// A callback's error as an `anyhow::Error`, keeping a raised error value
/// intact so a later `catch()` can still recover it. Anything else is
/// flattened to its rendering, as before.
pub(crate) fn callback_error(err: starlark::Error) -> anyhow::Error {
    if RaisedError::from_starlark(&err).is_some() {
        match err.into_kind() {
            starlark::ErrorKind::Native(e) | starlark::ErrorKind::Other(e) => return e,
            _ => unreachable!("from_starlark only matches Native and Other"),
        }
    }
    anyhow::anyhow!("{}", err)
}

/// `__test_future(value = ..., error = ...)`: a future that resolves to
/// `value`, or fails with `error` as the top of a two-link chain whose root
/// is "root cause". Lets the tests exercise `catch()` without a network.
#[cfg(test)]
mod test_future {
    use starlark::environment::GlobalsBuilder;
    use starlark::starlark_module;
    use starlark::values::{Heap, Value};

    use crate::engine::r#async::future::{FutureAlloc, StarlarkFuture};

    struct Resolved(String);

    impl FutureAlloc for Resolved {
        fn alloc_value_fut<'v>(self: Box<Self>, heap: Heap<'v>) -> Value<'v> {
            heap.alloc(self.0)
        }
    }

    #[starlark_module]
    pub(super) fn register(globals: &mut GlobalsBuilder) {
        fn __test_future<'v>(
            #[starlark(require = named)] value: Option<String>,
            #[starlark(require = named)] error: Option<String>,
        ) -> anyhow::Result<StarlarkFuture<'v>> {
            Ok(StarlarkFuture::from_future(async move {
                match error {
                    Some(msg) => Err(anyhow::anyhow!("root cause").context(msg)),
                    None => Ok(Resolved(value.unwrap_or_default())),
                }
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::axl_check;

    /// Starlark has no assertion builtin; the snippets share this one.
    const PRELUDE: &str = "def assert_eq(got, want):\n    if got != want:\n        fail(\"want\", repr(want), \"got\", repr(got))\n";

    fn ok(code: &str) {
        axl_check!(&format!("{PRELUDE}{code}")).unwrap_or_else(|e| panic!("expected ok, got: {e}"));
    }

    fn err(code: &str) -> String {
        axl_check!(code)
            .expect_err("expected evaluation to fail")
            .to_string()
    }

    const TYPES: &str = r#"
DeployError = error.type(fields = {"deployment": str, "attempt": attr(int, default = 1)})
DeployTimeout = DeployError.type(fields = {"after_ms": int})
Unrelated = error.type()
"#;

    fn with_types(body: &str) -> String {
        format!("{TYPES}\n{body}")
    }

    #[test]
    fn every_error_is_type_error_and_matches_its_whole_chain() {
        ok(&with_types(
            r#"
e = DeployTimeout("no ack", deployment = "prod", after_ms = 30000)
assert_eq(type(e), "error")
assert_eq(isinstance(e, DeployTimeout), True)
assert_eq(isinstance(e, DeployError), True)
assert_eq(isinstance(e, error), True)
assert_eq(isinstance(e, Unrelated), False)
assert_eq(isinstance(DeployError("x", deployment = "d"), DeployTimeout), False)
assert_eq(isinstance("x", error), False)
assert_eq(type(DeployError), "error_type")
assert_eq(isinstance(error("plain"), error), True)
"#,
        ));
    }

    #[test]
    fn annotations_accept_subtypes_and_reject_others() {
        ok(&with_types(
            r#"
def take(e: DeployError) -> str:
    return e.deployment

def any_error(e: error) -> str:
    return e.message

assert_eq(take(DeployTimeout("t", deployment = "prod", after_ms = 1)), "prod")
assert_eq(any_error(Unrelated("u")), "u")
"#,
        ));
        let msg = err(&with_types(
            r#"
def take(e: DeployError):
    pass

take(Unrelated("u"))
"#,
        ));
        assert!(
            msg.contains("does not match the type annotation `DeployError`"),
            "{msg}"
        );
    }

    #[test]
    fn fields_are_inherited_defaulted_and_type_checked() {
        ok(&with_types(
            r#"
e = DeployTimeout(message = "t", deployment = "prod", after_ms = 5)
assert_eq(e.message, "t")
assert_eq(e.deployment, "prod")
assert_eq(e.attempt, 1)
assert_eq(e.after_ms, 5)
assert_eq(str(e), "t")
assert_eq(repr(e), 'DeployTimeout(message = "t", deployment = "prod", attempt = 1, after_ms = 5)')
assert_eq(dir(e), ["after_ms", "attempt", "cause", "deployment", "message", "stacktrace"])
"#,
        ));
        let msg = err(&with_types(r#"DeployError("x", deployment = 3)"#));
        assert!(
            msg.contains("field `deployment` expected type `str`"),
            "{msg}"
        );
        let msg = err(&with_types(r#"DeployError("x")"#));
        assert!(msg.contains("missing required field `deployment`"), "{msg}");
        let msg = err(&with_types(
            r#"DeployError("x", deployment = "d", nope = 1)"#,
        ));
        assert!(msg.contains("unexpected field `nope`"), "{msg}");
        let msg = err(&with_types(r#"DeployError(deployment = "d")"#));
        assert!(msg.contains("missing its message"), "{msg}");
    }

    #[test]
    fn reserved_and_redeclared_fields_are_rejected() {
        let msg = err(r#"E = error.type(fields = {"cause": str})"#);
        assert!(msg.contains("`cause` is reserved"), "{msg}");
        let msg = err(&with_types(
            r#"E = DeployError.type(fields = {"deployment": str})"#,
        ));
        assert!(
            msg.contains("field `deployment` is already declared by DeployError"),
            "{msg}"
        );
    }

    #[test]
    fn cause_chains_errors() {
        ok(&with_types(
            r#"
root = error("disk full")
e = DeployError("upload failed", deployment = "prod", cause = root)
assert_eq(e.cause.message, "disk full")
assert_eq(e.cause.cause, None)
"#,
        ));
        let msg = err(r#"error("x", cause = "not an error")"#);
        assert!(msg.contains("cause must be an error or None"), "{msg}");
    }

    #[test]
    fn stacktrace_names_the_constructing_function() {
        // `error("x")` sits on the snippet's third line, after the prelude.
        let line = PRELUDE.lines().count() + 3;
        ok(&format!(
            r#"
def _build():
    return error("x")

frames = _build().stacktrace
names = [f.name for f in frames]
assert_eq("_build" in names, True)
f = [f for f in frames if f.name == "_build"][0]
assert_eq(f.path, "<snippet>")
assert_eq(f.line, {line})
"#
        ));
    }

    #[test]
    fn string_fail_is_unchanged() {
        let msg = err(r#"fail("oops", 1, False)"#);
        assert!(msg.contains("fail: oops 1 False"), "{msg}");
    }

    #[test]
    fn fail_raises_the_error_with_its_type_name() {
        let msg = err(&with_types(
            r#"fail(DeployError("no ack", deployment = "prod"))"#,
        ));
        assert!(msg.contains("DeployError: no ack"), "{msg}");
        assert!(msg.contains("Traceback"), "{msg}");
    }

    #[test]
    fn catch_passes_success_through_as_the_value() {
        ok(r#"
err, value = __test_future(value = "hello").catch().block()
assert_eq(err, None)
assert_eq(value, "hello")
"#);
    }

    #[test]
    fn catch_turns_a_runtime_failure_into_a_plain_error() {
        ok(r#"
err, value = __test_future(error = "request failed").catch().block()
assert_eq(value, None)
assert_eq(type(err), "error")
assert_eq(isinstance(err, error), True)
assert_eq(err.message, "request failed")
assert_eq(err.cause.message, "root cause")
assert_eq(err.cause.cause, None)
assert_eq(bool(err), True)
"#);
    }

    #[test]
    fn catch_with_types_reraises_other_errors_unchanged() {
        let msg = err(&with_types(
            r#"__test_future(error = "request failed").catch(DeployError).block()"#,
        ));
        assert!(msg.contains("request failed"), "{msg}");
    }

    #[test]
    fn catch_recovers_a_value_raised_in_a_callback() {
        ok(&with_types(
            r#"
def _raise(v):
    fail(DeployTimeout("slow", deployment = "prod", after_ms = 9))

err, value = __test_future(value = "x").map_ok(_raise).catch(DeployError).block()
assert_eq(value, None)
assert_eq(isinstance(err, DeployTimeout), True)
assert_eq(err.after_ms, 9)
assert_eq(err.deployment, "prod")
"#,
        ));
    }

    /// The caught value is the raised one, still live: its mutable fields
    /// stay usable, both through it and through anything else that holds them.
    #[test]
    fn caught_value_is_the_raised_value_and_stays_mutable() {
        ok(r#"
Collected = error.type(fields = {"items": list})
items = ["a"]
raised = Collected("bad", items = items)

def _raise(v):
    fail(raised)

err, _ = __test_future(value = "x").map_ok(_raise).catch().block()
assert_eq(err == raised, True)
err.items.append("b")
items.append("c")
assert_eq(raised.items, ["a", "b", "c"])
"#);
    }

    #[test]
    fn traceback_is_inherited_unless_overridden() {
        let msg = err(r#"
Refusal = error.type(traceback = False)
Specific = Refusal.type()
fail(Specific("nope"))
"#);
        assert!(
            !msg.contains("Specific:"),
            "a refusal renders its message alone: {msg}"
        );
        let msg = err(r#"
Refusal = error.type(traceback = False)
Loud = Refusal.type(traceback = True)
fail(Loud("nope"))
"#);
        assert!(msg.contains("Loud: nope"), "{msg}");
    }

    #[test]
    fn catch_must_come_last_once_and_take_error_types() {
        let msg = err(r#"__test_future(value = "x").catch().map_ok(str).block()"#);
        assert!(msg.contains("catch() must be the last call"), "{msg}");
        let msg = err(r#"__test_future(value = "x").catch().catch().block()"#);
        assert!(msg.contains("catch() was already called"), "{msg}");
        let msg = err(r#"__test_future(value = "x").catch(str).block()"#);
        assert!(msg.contains("catch() takes error types"), "{msg}");
    }
}
