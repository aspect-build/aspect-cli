# Directives

You can configure Aspect CLI using directives, which are specially-formatted
comments in `BUILD` files that govern behavior of the tool when visiting files
within the Bazel package rooted at that file.

## Go

Go directives for generating BUILD files are from the standard [gazelle go plugin](https://github.com/bazelbuild/bazel-gazelle#directives).

## JavaScript

JavaScript directives for generating BUILD files follow the same format as [gazelle](https://github.com/bazelbuild/bazel-gazelle). In addition to the generic directives from the [standard gazelle directives](https://github.com/bazelbuild/bazel-gazelle#directives) JavaScript (and TypeScript) specific directives are as follows:

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
| `# gazelle:js_tsconfig _filename_`                      | `tsconfig.json`             |
| Path to a `tsconfig.json` file used to help generate TypeScript rules.<br />This value is inherited by sub-directories and applied relative to each BUILD.<br />The `ts_project(tsconfig)` attribute is *NOT* set and must be done manually if necessary |
<!-- prettier-ignore-end -->
