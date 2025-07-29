# Starzelle BUILD generator

A BUILD generator where plugins implemented in Starlark can be used to generate BUILD files for bazel projects.

See [Starlark spec](https://github.com/bazelbuild/starlark/blob/master/spec.md), [core Starlark data types](https://bazel.build/rules/lib/core), [Starlark github-linguist](https://github.com/github-linguist/linguist/blob/v7.29.0/lib/linguist/languages.yml#L6831-L6852) for general Starlark docs and information.

See [Public Docsite](https://docs.aspect.build/cli/starlark/) for the plugin Starzelle API and documentation.

### Plugins via env

Additional plugins will be loaded from `${STARZELLE_PLUGINS}/*.lang.star` glob.

**FOR TESTING ONLY**: by default `STARZELLE_PLUGINS=${RUNFILES_DIR}/aspect_cli/gazelle/plugins/*.lang.star` for unit tests.

## TODO:

* logging API, builtin logging of some events/plugins/stages/?
* PrepareContext.properties access: https://github.com/aspect-build/silo/pull/5663#pullrequestreview-2103466655
* better error handling when plugins return bad data: https://github.com/aspect-build/silo/pull/5668#discussion_r1631761789
* change CLI config `configure.plugins.*` to support: plugin key/id, glob, references to external repos
