# blazon

A blazon is the formal description of a coat of arms. This one describes
Blaze's classes: it generates typed Rust bindings to Bazel's server classes
from a release's `A-server.jar`, one module per Bazel version, so a consumer
such as `crates/buildx` writes
`runtime.workspace()?.skyframe_executor()?.package_manager()?`
instead of JNI descriptors, cannot pass a `BlazeRuntime` where a `List<String>`
is expected, and sees shape changes between versions as a report rather than
as a `NoSuchMethodError`.

```
blazon --spec bindings.toml --out /tmp/bindings --bazel 9.0.1 --bazel 9.2.0
blazon --bazel 9.0.1 --dump net.starlark.java.eval.Starlark
```

Consumers do not check generated code in. A `build.rs` calls
`blazon::build_script(spec, out_dir)`, which binds the spec's `versions`,
fetching each release into the cache (`$BLAZON_BAZEL_CACHE`, else the user
cache's `buildx` directory) on first build, writes `out_dir/mod.rs` naming
each version module by absolute path, and prints the cargo rerun directives.
The crate then `include!`s `mod.rs`. Generated code depends on `blazon-rt`, and
with `starlark = true` on `blazon-starlark`, `starlark` and `allocative`.
The `release` feature (on by default) is the downloader and
`blazon::release`, the release cache `buildx` uses at runtime too; a consumer
that supplies a jar itself can turn default features off and call
`blazon::generate`. The `cli` feature is the binary.

`--bazel <version>` reads the release `buildx` already extracted into the user
cache; `--jar <path> --version <label>` binds any server jar; `--dump` prints a
class's members with descriptors and generic signatures, which is how a spec
entry is written. No JDK or `javap` is needed: class files are parsed with
`cafebabe`.

## Spec

```toml
versions = ["9.0.1", "9.2.0"]
starlark = true                       # bind Starlark types; see below

[[class]]
java = "com.google.devtools.build.lib.runtime.BlazeCommandDispatcher"
members = [
    "<init>(Lcom/google/devtools/build/lib/runtime/BlazeRuntime;)",
    "exec(Ljava/util/List;Ljava/lang/String;Lcom/google/devtools/build/lib/util/io/OutErr;)",
    "shutdown",
    "getWorkspace?",                  # may return null: `Result<Option<_>>`
    "put(Ljava/lang/Object;) as put_key",
    ".name as name_field",            # the field, when a method has the same name
    "put(Ljava/lang/Object;) nullable(0)",  # parameter 0 takes null though Bazel does not annotate it
    "@getName as name",               # Starlark attribute, for a class without @StarlarkMethod
]
```

A member is a name, a name with its parameter descriptors to pick one
overload, or `<init>(...)` for a constructor, optionally followed by
` as rust_name` and by `?` for a result that may be null. A leading `.` picks
the field when a method has the same name. An ambiguous bare name fails with
every overload listed. A class with `members = []` is opaque: it exists so
that signatures mentioning it are typed. A class that is not in the server jar
is an error naming the spec entry. Standard Java types (`java.lang`,
`java.util`, `java.io`, `java.math`, `java.time.Duration`) are
`blazon_rt::java`, are re-exported from every module, and are not listed; a
JDK class a selected signature needs that `blazon-rt` does not provide is an
error telling you to add it to `blazon-rt` and `src/jdk.rs`.

`--dump` prints, for a class, every field and method with descriptors,
generic signature and `@Nullable`, then its Starlark surface.

## Generated code

Each version becomes `v<version>.rs`, formatted with `prettyplease`, written
only when it changed. Every bound class is a handle `Name<'a, T..>` owning one
local reference, released when it drops, and valid on the thread that made it.
Java generics are phantom type parameters spelled `'static`
(`List<'a, JavaString<'static>>`); elements come out as `<T as Rebind<'a>>::Out`.
Call sites read like Rust:

```rust
let ws = runtime.workspace()?;                      // getWorkspace(), `get_` dropped
let label = Label::parse_canonical(&mut env, "//a:b")?;
module.set_clock(Some(&clock))?;                    // private field, through reflection
for e in map.entries()? { let e = e?; ... }
```

- Names are snake case; `getFoo()` with no parameters is `foo()`, `isX`, `hasX`
  and `setX` stay. Overloads need ` as <name>`; names every handle already has
  (`upcast`, `clone`, ...) are rejected.
- `String` is `&str` in and `String` out; `boolean` and the numbers are
  scalars; `Boolean`, `Integer` and `Long` are scalars; `Optional<T>` is
  `Option`; `byte[]` is `&[u8]`/`Vec<u8>`, `String[]` and reference arrays are
  slices and `Vec`s. Other arrays are an error naming the member.
- A `@Nullable` result (a declaration or type annotation in the class file) or
  a `?` in the spec is `Option`; any other null result is an error naming the
  member. A `@Nullable` parameter is `Option<&_>`.
- Parameters that take a bound class are `&impl IsA<Class>`; a `?` or `? super X` type argument in a parameter makes the member generic over it (`mixed<W0>(&impl IsA<List<'static, W0>>)`), so any instantiation is accepted; every handle is
  `IsA` of its transitive supertypes in the module and in `blazon_rt::java`,
  with `.upcast::<T>()`, checked `.downcast::<T>()`, `From<Sub> for Super` and
  `TryFrom<Super> for Sub`.
- `static final` primitives and `String`s with a `ConstantValue` are associated
  `const`s. Other statics and constructors are associated functions taking the
  `JNIEnv` they make references in; generic factories take their type
  parameters (`ImmutableMap::<K, V>::empty(env)`).
- Members that are not public are reached through `java.lang.reflect` (the
  `Method`, `Field` or `Constructor` is looked up once); a non-public instance
  field also gets `set_<name>`, final or not. Primitives cross as boxed values
  in `boolean`, `int` and `long` only.
- Handles are `Debug`/`Display` through `toString`, `Clone` (a new local
  reference), `Eq`/`Hash` through `equals`/`hashCode`, and `Ord` for
  `Comparable` classes.
- Every call goes through the class and member IDs cached in `blazon_rt::member`
  and returns `blazon_rt::Result`; a Java exception is a `blazon_rt::Error` with
  the trace as its `source()`.

`mod.rs` lists every generated version in `SUPPORTED`, and with more than one
version a shape report on stderr says which members are the same, changed or
missing per version, including the Starlark surface.

## Starlark types

With `starlark = true` (or `starlark = true` on a class; `false` opts one out),
each bound class also gets a Starlark value type `NameValue` holding a global
reference, registered by the module's `register()`. Its methods are exactly the
class's `@StarlarkMethod`s as `CallUtils` collects them from the class and its
ancestors (a `structField` is an attribute), with parameters from `@Param`:
`allowedTypes` become a union (`Either` for the Rust unpacker, a union for the
typechecker), `defaultValue` a default, `named`/`positional` are honoured, and
`useStarlarkThread` / `useStarlarkSemantics` are supplied by the facet. A class
without annotations exposes its selected instance members under their Java
names (`@name` in the spec makes one an attribute named by its Rust name), and
gets iteration, `len`, `in` and indexing from `Iterable`/`Collection`/`List`/`Map`
ancestors. The Starlark interfaces a class implements give attribute access,
iteration, indexing, `in`, calls, `len`, truthiness, and equality and hashing
through the Java ones. Every in-jar class a selected signature or
`allowedTypes`/`generic1` names is bound too, opaque. Methods with `*args`,
`**kwargs`, `selfCall` or a default that is not a literal are listed in the
generated docs as not exposed.

The Starlark runtime members the facets call (`Starlark.str`, `getattr`,
`StarlarkThread.createTransient`, ...) are bound implicitly and checked per
version, so a moved one is a generation error.

## Tests

`tests/fixture` is a small Java source tree built into `fixture.jar`
(`sh tests/fixture/build.sh`) exercising generics, wildcards, arrays, varargs,
nullability, inner classes, overloads, statics, constants, private members and
Starlark annotations. `tests/fixture.rs` compares the emitted code with
`tests/snapshots/fixture.rs.txt` (`BLAZON_BLESS=1` rewrites it) and compiles it
against `blazon-rt` and `blazon-starlark`; `crates/blazon-selftest` runs the
generated code in a JVM, against the fixture and, when a Bazel 9.0.1 release is
cached, against Bazel itself.

## Bazel build

Under Bazel the build script runs in a sandbox without network, so the server
jar has to be supplied as an input rather than downloaded; `BLAZON_BAZEL_CACHE`
pointed at a directory laid out like the user cache (`bazel/<version>/A-server.jar`)
is the intended seam. This is not solved yet; the `BUILD.bazel` files wire a
`cargo_build_script` with the spec as data and are untested.
