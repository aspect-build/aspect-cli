# Directives

You can configure Aspect CLI using directives, which are specially-formatted
comments in `BUILD` files that govern behavior of the tool when visiting files
within the Bazel package rooted at that file.

## Go

Go directives for generating BUILD files are from the standard [gazelle directives].

## JavaScript

JavaScript directives for generating BUILD files follow the same format as gazelle.
You can use generic directives from the [gazelle directives], as well as the following JS/TS
specific directives.

TypeScript source files are those ending in `.ts`, `.tsx` as well as `.js`, `.mjs`.
Test source files are source files ending with `.spec.ts` (and other ts extensions).
The test file pattern can be configured with the 'js*test*\*' directives.

By default `aspect configure` creates new BUILD files for each directory containing source files.
This can be configured to only edit existing BUILD files using the `js_generation_mode` directive.

Each BUILD file may have a `ts_project` rule for sources, another for tests,
a `npm_package` rule for pnpm workspace projects, and `npm_link_all_packages` for linking node_modules.
Which rules are configured depends on the source files and directives that apply.

Next, all source files are collected into the `srcs` of the `ts_project`,
either the primary rule or tests rule.

Finally, the `import` statements in the source files are parsed, and
dependencies are added to the `deps` attribute of the appropriate
`ts_project` rule which the source file belongs to.

Dependencies may also be found other ways such as from the CommonJS `require` function.

If a `package.json` file exists declaring npm dependencies, a `npm_link_all_packages` rule
is generated for declaring depending on individual NPM packages.

If the `package.json` is a pnpm workspace project a `npm_package` rule may be generated to
enable other projects to declare dependencies on the package.

<!-- prettier-ignore-start -->
| **Directive**                                           | **Default value**           |
| ------------------------------------------------------- | --------------------------- |
| `# gazelle:js enabled\|disabled`                        | `enabled`                   |
| Enable the JavaScript directives. |
| `# gazelle:js_generation_mode none\|directory`          | `directory`                 | 
| Enable generation of new BUILD files within each directory, or do not generate and only modify existing BUILD files. |
| `# gazelle:js_pnpm_lockfile _lockfile_`                 | `pnpm-lock.yaml`            |
| Path to the `pnpm-lock.yaml` file containing available npm packages. <br />This value is inherited by sub-directories and applied relative to each BUILD. |
| `# gazelle:js_ignore_imports _glob_`                    |                             |
| Imports matching the glob will be ignored when generating BUILD files in the specifying directory and descendants. |
| `# gazelle:js_resolve _glob_ _target_`                  |                             |
| Imports matching the glob will be resolved to the specified target within the specifying directory and descendants.<br />This directive is an extension of the standard `resolve` directive with added glob support and only applying to JavaScript rules. |
| `# gazelle:js_validate_import_statements error\|warn\|off`   | `error`                      | 
| Ensure all import statements map to a known dependency. |
| `# gazelle:js_project_naming_convention _name_`         | `{dirname}`                 |
| The format used to generate the name of the main `ts_project` rule. |
| `# gazelle:js_tests_naming_convention _name_`           | `{dirname}_tests`           |
| The format used to generate the name of the test `ts_project` rule. |
| `# gazelle:js_files _glob_`                             | `**/*.{ts,tsx}`             |
| A glob pattern for files to be included in the main `ts_project` rule.<br />Multiple patterns can be specified by using the `js_files` directive multiple times.<br />When specified the inherited configuration is replaced, not extended. |
| `# gazelle:js_test_files _glob_`                        | `**/*.{spec,test}.{ts,tsx}` |
| Equivalent to `js_files` but for the test `ts_project` rule. |
| `# gazelle:js_npm_package_target_name _name_`           | `{dirname}`                 |
| The format used to generate the name of the `npm_package` rule. |
| `# gazelle:js_tsconfig enabled\|disabled`               | `enabled`                   |
| Enable generation of `ts_config` rules.<br />This value is inherited by sub-directories and applied relative to each BUILD.<br />The `ts_project(tsconfig)` attribute is *NOT* set and must be done manually if necessary |
| `# gazelle:js_custom_files _name_ _glob_`               |                             | Generate additional custom `ts_project` targets |
| `# gazelle:js_custom_test_files _name_ _glob_`          |                             | Generate additional custom `ts_project` testonly targets |
<!-- prettier-ignore-end -->

[gazelle directives]: https://github.com/bazelbuild/bazel-gazelle#directives
