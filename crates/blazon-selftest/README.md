# blazon-selftest

Not published. Generates bindings and Starlark types for the `blazon` fixture jar
and, when Bazel 9.0.1 is in the buildx cache, for Bazel itself, and runs them in
a JVM (`tests/`). `cargo run -p blazon-selftest --example bench` measures call
overhead.
