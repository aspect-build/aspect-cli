# Aspect Gazelle Runner

An enhanced version of the `gazelle_binary()` rule providing:
* enable/disable languages at runtime instead of at build time
* gitignore support
* 
* opentelemetry tracing support
* watch protocol support
* caching of gazelle source code analysis
* dx enhancements including:
    * stats outputted to the console
    * progress/status reporting

Today these features are available when running via `aspect configure`.

See `../main.go` or experimental standalone binary.
