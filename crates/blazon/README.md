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

### Per-version members

A release can change a signature the spec names, so a member (or a whole entry) can be
bound in some Bazel versions only:

```toml
[[class]]
java = "com.google.devtools.build.lib.buildeventservice.client.BuildEventServiceGrpcClient"
members = [
    "<init>(Lio/grpc/ManagedChannel;Lio/grpc/CallCredentials;Lio/grpc/ClientInterceptor;Ljava/lang/String;Ljava/util/UUID;) when 9.0.1",
    "<init>(Lio/grpc/ManagedChannel;Lio/grpc/CallCredentials;Lio/grpc/ClientInterceptor;) when 9.2.0",
]

[[class]]
java = "pkg.OnlyInNewer"
versions = ["9.2.0"]              # also on [[implement]]
```

` when 9.0.1, 9.2.0` ends a member, after `as`, `?` and `nullable(..)`. A member with a
condition is generated only in the matching version modules (each version is its own
module, so code written against `v9_0_1` is unaffected) and the shape report prints it as
`only in 9.0.1  Class.member`. A member without a condition that a version lacks is still a
generation error, so a conditional member is the only way to say the difference is intended.
A class or implementation left out of a version by `versions` shows as `missing` there.

## Option schema

An `[options]` section makes every version module carry Bazel's flag schema, read
from the `@Option` annotations of every class that extends
`com.google.devtools.common.options.OptionsBase`, and nothing else:

```toml
[options]
fragments = true                        # every FragmentOptions subclass (a configuration fragment)
include_effect_tags = ["LOADING_AND_ANALYSIS", "BUILD_FILE_SEMANTICS", "AFFECTS_OUTPUTS"]
exclude_effect_tags = ["EXECUTION", "NO_OP", "TERMINAL_OUTPUT"]
exclude_metadata_tags = ["INTERNAL"]    # the default
include_classes = ["com.google.devtools.build.lib.pkgcache.LoadingOptions"]
exclude_classes = ["com.google.devtools.build.lib.remote.*"]   # a trailing `*` is a prefix
include = ["some_flag"]                 # by option name
exclude = ["other_flag"]
```

Rules, in order; the first that applies decides:

1. `exclude` names the option: out.
2. `include` names the option: in.
3. The class is in `exclude_classes`: out.
4. The option carries a metadata tag in `exclude_metadata_tags`: out.
5. The class is in `include_classes`: in.
6. The class is a fragment and `fragments` is true: in, whatever its effect tags.
7. Otherwise the option is in when it has an effect tag in `include_effect_tags`
   and not all of its tags are in `exclude_effect_tags`.

The aim is every flag that changes Starlark semantics or Skyframe state, so a
fragment's flags are in even when `EXECUTION` is among their tags. Two options
with the same name in different classes are an error listing every clash; resolve
it with `exclude_classes`.

Each option becomes an `OptionDef` in `OPTIONS: &[OptionDef]`, a `static` sorted by
name with no allocation at startup, inside `pub mod options` of the version module
(`option(name)` binary-searches it; `closest(name)` returns suggestions for a typo).
`bindings::options("9.0.1")` returns the table of a version. `Kind` is `Bool`, `Int`,
`Long`, `Double`, `String`, `Label`, `Path`, `Duration`, `Void`, `Enum(&[constants])`,
`List(&Kind)`, `Dict(&Kind)`, or `Opaque(converter class)` for a converter blazon does
not know; an enum keeps its constants even then, and enum values are matched
case-insensitively by Bazel. Name, default, help and tags are Bazel's own text.
`--dump-options` prints what the spec selects with counts per class and kind, the
`Opaque` options and the ones without help; `--dump-raw-options` prints everything
discovered. The shape report lists each option as `same` or `differs` across versions.

buildx's spec adds these by name, because they sit in classes the rule excludes
(`runtime.CommonCommandOptions`, `BazelRulesModule$AllCommandGraveyardOptions`,
`buildtool.BuildRequestOptions`) but change what Starlark or Skyframe sees:

- `repo_env`: environment variables for repository rules.
- `incompatible_repo_env_ignores_action_env`: whether `--action_env` also reaches repository rules.
- `experimental_strict_repo_env`: whether repository rules see the full client environment.
- `enable_bzlmod`: whether module resolution runs.
- `enable_workspace`: whether the WORKSPACE file is loaded.
- `incompatible_use_plus_in_repo_names`: the spelling of canonical repository names.
- `experimental_enable_aspect_hints`: whether the `aspect_hints` attribute is honoured.
- `aspects_parameters`: parameter values every command-line aspect is analysed with.

Considered and left out: `http_*` (download tuning), `enable_platform_specific_config`
and `workspace_status_command` (client and stamping concerns), `symlink_prefix`
(output layout), `incompatible_macos_set_install_name` (C++ link detail).

## Implementing Java types from Rust

Bazel calls into its own extension points (a `SpawnStrategy`, the C++ include
scanning contexts), so Rust has to be able to implement a Java interface or
abstract class. blazon does it with generated class files and no Java compiler:

```toml
[[implement]]
java = "com.google.devtools.build.lib.rules.cpp.CppIncludeExtractionContext"
rust = "IncludeContext"             # the trait's name; default `<Handle>Impl`
members = ["getArtifactResolver?"]  # which abstract methods, as for a class; default all
extends = "pkg.AbstractBase"        # optional: a class the implementation extends
```

`java` is an interface, or an abstract (or any non-final public) class to extend;
`extends` adds a base class to an interface. The type is bound on the spec's behalf
when no `[[class]]` lists it, so `Name::from_impl` has a handle to return; list it to
select members to call. A JDK type cannot be implemented (its handle is in `blazon-rt`).
`members` selects abstract methods by `name` or `name(<descriptors>)` with ` as rust_name`
and a trailing `?` for a result that may be null; the abstract methods it leaves out
throw `AbstractMethodError` when called. Default and static methods are never
implemented: the JVM runs the interface's own.

Per type the module has a trait, and a constructor on the handle:

```rust
pub trait CppIncludeExtractionContextImpl: Send + Sync {
    fn artifact_resolver<'a>(&self, env: &mut JNIEnv<'a>) -> blazon_rt::Result<ArtifactResolver<'a>>;
}

let ctx = CppIncludeExtractionContext::from_impl(&mut env, Box::new(MyContext { .. }))?;
```

Each abstract method is one trait method, with the types of a call into Java read the
other way: what Java passes is what a call would return (`String`, `Option<String>`
for `@Nullable`, `i32`, handles, `Vec<u8>`, `Option<_>` for `Optional`), and the
return value is what a call would take. Class type variables and method type
variables are `Object`, because the descriptor erases them. `Err` becomes a thrown
`RuntimeException` whose message is `Handle.method: <error>` and whose cause carries the
rest of the error chain; a panic is caught and thrown as an `IllegalStateException`.
It never unwinds into the JVM.
An error that came from a Java exception, or was made from one (`Err(SpawnExecException::new(..)?.into())`,
or any `T: IsA<Throwable>` with `Error::from_throwable`), keeps the exception object, and the
trampoline throws that very object, whatever checked exceptions the method declares; so a Rust
implementation that delegates to Java and propagates with `?` throws what Java threw.

How it works, so a failure can be read: blazon writes a class file per implemented type,
`blazon/impl/<Handle>$<hash>` (Java 8 format, no branches, so no stack maps), with a `long
handle` field, a package-private `(J)V` constructor, and one `public native` method per
abstract method with the exact erased descriptor, plus the shared `blazon/impl/Free`. The
first `from_impl` defines them with `DefineClass` in the class loader of the implemented type
(so they see Bazel's classes) and registers the natives, one `extern "system"` function per
method. The Rust value is a `Box` whose address is the `handle`. The constructor registers the
object with a `java.lang.ref.Cleaner`, which drops the box once the object is unreachable.
A running call has the object in its native frame, so the value outlives every call in
flight, and it is dropped on the cleaner thread: the trait is `Send + Sync`, and calls
arrive on whatever thread Bazel uses, which is attached for the call.

Limits: the base class needs a public or protected no-argument constructor; every
implemented method must be public or protected; the generated class is `final` and cannot be
subclassed; only abstract methods are implemented, not overrides of concrete ones; the
`handle` constructor is package-private, but reflection could still call it with a bad
pointer.

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
without annotations exposes its selected instance members under their Rust names,
the same as the generated Rust API (`getWorkspace` is `workspace`, `setClock` is
`set_clock`, ` as name` overrides), so a script never sees a Java-style name;
`@name` in the spec makes one an attribute instead of a method. A class with
`@StarlarkMethod`s keeps the names Bazel declared. It also
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
