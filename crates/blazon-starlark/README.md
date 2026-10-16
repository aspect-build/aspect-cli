# blazon-starlark

Runtime for the Starlark types `blazon` generates with `starlark = true`.

- `register`, `Registry`: a generated version module registers itself once per
  process; `wrap` and `unwrap` convert between Java objects and Starlark values
  through it.
- Java to Starlark copies only `String`, `Boolean`, `Integer`, `Long`,
  `StarlarkInt` and null/`NoneType`. Everything else is the most specific
  generated type, one of the JDK views, or an error naming the class.
- `ToJava`, `JavaValue`: what can be passed to Java, and what a generated value
  hands back.
- `jdk`: Starlark views of `java.util` lists, sets, maps and collections.
