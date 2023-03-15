# Directives

You can configure Aspect CLI using directives, which are specially-formatted
comments in `BUILD` files that govern behavior of the tool when visiting files
within the Bazel package rooted at that file.

## JavaScript

JavaScript directives follow the same format as [gazelle](https://github.com/bazelbuild/bazel-gazelle#directives).

Directives specific to JavaScript (and TypeScript) are as follows:

| **Directive**                   | **Default value**           | **Param(s)**        |
| ------------------------------- | --------------------------- | ------------------- |
| `js`                            | `enabled`                   | `enabled\|disabled` |
| `js_generation_mode`            | `directory`                 | `none\|directory`   |
| `js_pnpm_lockfile`              | `pnpm-lock.yaml`            | _lockfile_          |
| `js_ignore_imports`             |                             | _glob_              |
| `js_resolve`                    |                             | _glob_ _target_     |
| `js_validate_import_statements` | `true`                      | `true\|false`       |
| `js_project_naming_convention`  | `{dirname}`                 | _name_              |
| `js_tests_naming_convention`    | `{dirname}_tests`           | _name_              |
| `js_files`                      | `**/*.{ts,tsx}`             | _glob_              |
| `js_test_files`                 | `**/*.{spec,test}.{ts,tsx}` | _glob_              |
| `js_npm_package_target_name`    | `{dirname}`                 | ...                 |

All JavaScript directives are specified via `gazelle` such as:

```
# gazelle:js enabled
```
