---
title: BUILD file generators in Starlark
sidebar_label: Starlark Extensions
---

Aspect CLI includes the ability to write BUILD file generation extensions in Starlark.
These extensions are executed when a user runs [`aspect configure`](./commands/aspect_configure.md).

Watch [Master Build File Automation with Aspect CLI and Gazelle](https://www.youtube.com/embed/pmveB5OfuLg?si=1GczyKvxtFpvl0hU).

> [!WARNING]
> This feature is experimental, and the Starlark API is subject to change without notice.

## Why we made it

Compared with [writing Gazelle extensions in Go](https://github.com/bazelbuild/bazel-gazelle/blob/master/extend.md), there are numerous advantages:

### It's pre-compiled and statically linked

1. Starlark is an interpreted language, so there's no need to recompile a binary when code changes.
   Product engineers are never slowed down waiting for compilation, and aren't affected by problems
   with building Go code.
1. Some extensions require CGo, like rules_python.
   This requires a functional C++ toolchain on every users machine, making it even less portable
   or forcing you to setup a hermetic C++ toolchain, including a giant sysroot download, even in
   repositories that have no C++ code.
   See https://github.com/bazelbuild/rules_python/issues/1913

### Starlark is the language of Bazel extensibility

1. Logic can be shared between a rule implementation and the corresponding BUILD generator.
   Also, logic implemented in a macro that provides a user experience like `my_abstraction`
   can be ported to a generator which writes the equivalent targets into the `BUILD` file
   (imagine this as "inline macro" refactoring) - and vice versa.
1. All developers interacting with Bazel have basic Starlark familiarity and can read the code. Not everyone knows Go.
1. It's much easier to customize the logic in a user's repository, obviating the need for more expressive "directives" which are load-bearing comments that are easy to miss and don't get syntax highlighting.

### It's approachable

1. Our API is designed to be easy for novices to use. In contrast, the effort to implement and ship a Gazelle extension is high because the API abstractions are low-level.
1. Writing and sharing a general-purpose Gazelle extension is difficult because it's expected to handle every possible scenario. In your repo you can make a tradeoff to take shortcuts based on your needs.

## Design

Aspect CLI embeds a starlark interpreter as a Gazelle "extension".
Inside this interpreter a new top-level symbol `aspect` is exposed which gives access to the API.

This allows existing Gazelle extensions written in Go to interoperate with Starlark extensions.
Currently those other Go extensions must be statically compiled into the `aspect` binary, however
we anticipate that https://github.com/bazelbuild/bazel-gazelle/issues/938 will allow pre-compiled
custom Gazelle extensions to participate under `aspect configure`.

## Writing plugins

Create a starlark source file.
We recommend using a `.star` extension, so that [GitHub](https://github.com/github-linguist/linguist/blob/559a6426942abcae16b6d6b328147476432bf6cb/lib/linguist/languages.yml#L6770-L6772) and other tools will provide syntax highlighting, formatting, etc.

Typical locations include
-  `/tools/configure/my_extension.star`: next to other tool setup
-  `/bazel/rules_mylang.star`: next to Bazel-specific support code
-  `/.aspect/cli/my_ruletype.star`: alongside configuration of Aspect CLI

The plugin will use the `aspect` top-level symbol we provide in the Starlark interpreter context.
You'll call `aspect.register_configure_extension` at minimum.

Here's a very simple example that generates `sh_library` targets for all Shell scripts:

```starlark
"Create sh_library targets for .bash and .sh files"

aspect.register_configure_extension(
    id = "rules_sh",
    prepare = lambda cfg: aspect.PrepareResult(
        sources = aspect.SourceExtensions(".bash", ".sh"),
    ),
    declare = lambda ctx: ctx.targets.add(
        kind = "sh_library",
        name = "shell",
        attrs = {
            "srcs": [s.path for s in ctx.sources],
        },
    ),
)
```

See a basic [rules_cc example](https://github.com/aspect-build/codelabs/blob/12dd55cbae7612d9c7253a7d27c932f1291ffadf/.aspect/cli/rules_cc.star)
currently using basic regular expressions to detect `#include` statements and `main()` methods to generate `cc_library` and `cc_binary` targets.

We plan to provide more examples in the future. For now, consult the API docs below.
  
## Loading plugins

The starlark interpreter runtime is shipped in [Aspect CLI](./index.mdx).
Check that page for install instructions first.

Next, add a section in the `.aspect/cli/config.yaml`:

```yaml
configure:
    plugins:
        WORKSPACE_relative/path/to/my_plugin.star
```

## Enabling plugins

Individual plugins can be enabled/disabled via BUILD directives:
```
# aspect:{plugin_id} enabled|disabled
```

## Extension registration API

### `aspect.register_rule_kind`

Register a new rule kind that may be generated by a `configure` extension.

*Args*:
- `name`: the name of the rule kind
- `From`: the target .bzl file that defines the rule
- `NonEmptyAttrs`: a set of attributes that, if present, disqualify a rule from being deleted after merge.
- `MergeableAttrs`: a set of attributes that should be merged before dependency resolution
- `ResolveAttrs`: a set of attributes that should be merged after dependency resolution

### `aspect.register_configure_extension`

Register a `configure` extension for generating targets in `BUILD` files.

*Args*:
- `name`: a unique identifier for the extension, may be referenced in Starlark API or used in `# aspect:{name} enabled|disabled` directives etc
- `properties`: a map of name:property definitions (optional), see [Extension Properties](#extension-properties) and `aspect.Property`
- `prepare`: the prepare stage callback (optional)
- `analyze`: the analyze stage callback (optional)
- `declare`: the declare stage callback (optional)

## Extension Properties

Property values can be set in `BUILD` files using `# aspect:{name} {value}` directives.

Each stage has access to the extension properties using `ctx.properties`.

Property values are inherited from parent packages.

**aspect.Property(type, default)**:

Construct a property definition.

Args:
* `type`: the property type, one of `string`, `[]string`, `number`, `bool`
* `default`: the default value for the property (optional)

## Stages

Starzelle has multiple stages for generating `BUILD` files which extensions can hook into:

1. Prepare
2. Analyze
3. Declare

All stages are optional for extensions.

Stages are executed per `BUILD` file. `BUILD` files may or may not be pre-existing depending on the `# aspect:generation_mode update_only|create_and_update`.

Stages are executed in sequence, however within a stage extensions may be executed in parallel.

### Prepare

```Prepare(ctx PrepareContext) PrepareResult```

Declares which files the extension will process and any queries to run on those files.

**PrepareContext**:

The context for a `Prepare` invocation.

Properties:
* `.repo_name`: the name of the Bazel repository
* `.rel`: the directory being prepared relative to the repository root
* `.properties`: a name:value map of extension property values configured in `BUILD` files via `# aspect:{name} {value}`

**aspect.PrepareResult(sources, queries)**:

The factory method for a `Prepare` result.

Args:
* `sources`: one or a list of source file matcher(s)
* `queries`: a `name:aspect.*Query` map of queries to run on matching files, see [Query Types](#query-types)

#### Source Matchers

**aspect.SourceFiles(files...)**:

Match specific file paths.

**aspect.SourceExtensions(exts...)**:

Match files with the trailing extensions. Extensions should include the leading `.`.

**aspect.SourceGlobs(patterns...)**:

Match files matching glob patterns. Note that globs are significantly slower than exact
paths or extension based matchers.

### Analyze

```Analyze(ctx AnalyzeContext) error```

Analyze source code query results and potentially declare symbols importable by rules.

**AnalyzeContext**:

Properties:
* `.source`: a `aspect.TargetSource` of the source file being analyzed

Methods:
**`.add_symbol(id, provider_type, label)`**: add a symbol to the symbol database.

Args:
* `id`: the symbol identifier
* `provider_type`: the type of the provider such as "java_info" for java packages etc
* `label`: the Bazel label producing the symbol

#### Types

**aspect.TargetSource**:

Metadata about a source file being analyzed.

Properties:
* `.path`: the path to the source file relative to the `BUILD`
* `.query_results`: a `name:result` map for each query run on this source file

See [Query Types](#query-types) for more information on query result types.

**aspect.Label(repo, pkg, name)**

Construct a Bazel label.

Args:
* `repo`: the repository name (optional)
* `pkg`: the label package (optional)
* `name`: the label name

### DeclareTargets

```DeclareTargets(ctx DeclareTargetsContext) DeclareTargetsResult```

Declare targets to be generated in the `BUILD` file given the declaration context

**DeclareTargetsContext**:

The context for a `DeclareTargets` invocation.

Properties:
* `.repo_name`: the name of the Bazel repository
* `.rel`: the directory being prepared relative to the repository root
* `.properties`: a name:value map of extension property values configured in `BUILD` files via `# aspect:{name} {value}`
* `.sources`: a list of `aspect.TargetSource`s to process based on the `prepare` stage results
* `.targets`: actions to modify targets in the `BUILD` file, see `aspect.DeclareTargetActions`

**DeclareTargetActions**:

Actions to add/remove targets for a `BUILD` file.

Methods:
* `.add(name, kind[, attrs][, symbols])`: add a rule of the specified kind to the `BUILD` file with a set of attributes and exported symbols
 Params:
  * `name`: the name of the rule
  * `kind`: the rule kind, a native/builtin rule or one registered with `aspect.register_rule_kind`
  * `attrs`: a name:value map of attributes for the rule, values of type `aspect.Import` will be resolved to Bazel labels
  * `symbols`: a list of symbols exported by the rule
* `.remove(name)`: remove a rule from the BUILD file

**aspect.Import()**:

A placeholder for a Bazel label that will be resolved after the declare stage.

When an attribute value (or value within an array) is an `aspect.Import` it
will be resolved after the declare stage and potentially be replaced with a Bazel label.

If the import is resolved to the same target (a self reference) it will be removed from the attribute.
If the import is not resolved an error will be thrown unless the import is declared as optional.

Args:
* `id`: the symbol identifier
* `provider`: the symbol type being imported. Imported symbols must have the same symbol type as the rule defining the symbols such as `js` for the JS/TS `configure` extension.
* `optional`: whether the import is optional and should be ignored if not found
* `src`: the source of the import (optional). Only used for debugging and error messages.

## Query Types

Source files can be queried using various methods to extract information for analysis. Some query types return data
directly from the source code, such JSON and other structured data, while others return `QueryMatch` objects describing
the matched content.

**aspect.AstQuery(grammar, filter, query)**:

The factory method for an `AstQuery`.

Args:
* `filter`: a glob pattern to match file names to query
* `grammar`: the tree-sitter grammar to parse source code as (optional, default based on file extension)
* `query`: a tree-sitter query to run on the source code AST

A [tree-sitter](https://tree-sitter.github.io/tree-sitter/) query to run on the parsed AST of the file.

See [tree-sitter pattern matching with queries](https://tree-sitter.github.io/tree-sitter/using-parsers#pattern-matching-with-queries)
including details such as [query syntax](https://tree-sitter.github.io/tree-sitter/using-parsers#query-syntax),
[predicates](https://tree-sitter.github.io/tree-sitter/using-parsers#predicates) for filtering,
[capturing nodes](https://tree-sitter.github.io/tree-sitter/using-parsers#capturing-nodes) for extracting `QueryMatch.captures`.

The query result is a list of `QueryMatch` objects for each matching AST node. Tree-sitter capture nodes
are returned in the `QueryMatch.captures`, the `QueryMatch.result` is undefined.

**aspect.RegexQuery(filter, expression)**:

The factory method for a `RegexQuery`.

Args:
* `filter`: a glob pattern to match file names to query
* `expression`: a regular expression to run on the file

The query result is a list of `QueryMatch` objects for each match in the file.

Regex capture groups are returned in the `QueryMatch.captures`, keyed by the capture group name.
For example, `import (?P<name>.*)` will populate `QueryMatch.captures["name"]` with the captured value.

The full match is returned in the `QueryMatch.result`.

See the [golang regex](https://pkg.go.dev/regexp) documentation for more information.

**aspect.RawQuery(filter)**:

The factory method for a `RawQuery`.

Args:
* `filter`: a glob pattern to match file names to return

The query result is the file content as-is with no parsing or filtering.

**aspect.JsonQuery(filter, query)**:

The factory method for a `JsonQuery`.

Args:
* `filter`: a glob pattern to match file names to query
* `query`: a JQ filter expression to run on the JSON document

The query result is a list of each matching JSON node in the document.

For queries designed to return a single result the result will be an array of one object, or empty array if no result is found.

JSON data types are represented as golang primitives and basic arrays and maps, see [json.Unmarshal](https://pkg.go.dev/encoding/json#Unmarshal).

See the [jq manual](https://jqlang.github.io/jq/manual/#basic-filters) for query expressions.
See [golang jq](https://github.com/itchyny/gojq) for information on the golang jq implementation used by starzelle.

**aspect.YamlQuery(filter, query)**:

The factory method for a `YamlQuery`.

Args:
* `filter`: a glob pattern to match file names to query
* `query`: a YQ filter expression to run on the YAML document

The query result is a list of each matching YAML node in the document.

For queries designed to return a single result the result will be an array of one object, or empty array if no result is found.

YAML queries are implemented using the [yq](https://mikefarah.gitbook.io/yq) tool which borrows syntax from `jq`.

See the [jq manual](https://jqlang.github.io/jq/manual/#basic-filters) for query expressions.

**aspect.QueryMatch**:

The result of a query on a source file.

Properties:
* `.result`: the matched content from the source file such as raw text
* `.captures`: a `name:value` map of captures from the query

## Utils

#### `path.join(parts...)`

Joins one or more path components intelligently.

#### `path.dirname(p)`

Returns the dirname of a path.

#### `path.base(p)`

Returns the basename (i.e., the file portion) of a path, including the extension.

#### `path.ext(p)`

Returns the extension of the file portion of the path.
