# AXL Load Semantics

AXL scripts use the `load` statement to import symbols from other `.axl` files, similar to Starlark's load functionality. This allows modular scripting while enforcing security and resolution rules to keep loads within the repository boundaries.

## Supported Load Path Types

- **Relative Paths**: Start with `./` or `../` (e.g., `load("./utils.axl")`).
  - Resolved relative to the directory of the current script.
  - Supports relative loads within the repository, but prevents escaping the repository root.
  - Supports relative relative within a module, but prevents escaping the module root.

- **Repository/Module-Root Relative Paths**: No leading `./` or `/` (e.g., `load("path/to/script.axl")`).
  - Treated as paths starting from the repository root or module root.

- **Module Paths**: Start with `@` followed by the module name (e.g., `load("@module/path.axl")`).
  - Loads a path from a module.
  - The path after the module name is relative within that module.
  - No path traversal (e.g., `..`) is allowed in the module-relative part.

## Behavior When Loading from Within a Module

- If your script is located inside a vendored module, loads are scoped to stay within that module's directory for relative or repo-relative paths.
- If a direct file is not found, the module loader will scan parent directories for a matching vendored module file until the repository root is reached.
- Module paths (`@`) can be used to access other modules from within a module.

## Restrictions and Safety Rules

- **Path Validity**:
  - Paths must not start with `/` (no operating system absolute paths).
  - Paths must not contain `//` (double slashes).
  - All resolved paths must remain within the repository root; attempts to escape will fail.

- **Module-Specific Rules**:
  - When inside a module, you cannot load files outside the module using relative or repo-relative paths.
  - Resolved paths cannot contain multiple vendor segments (to prevent nested or invalid vendor access).

- **Cycle Prevention**: Recursive loads that form cycles (e.g., A loads B, B loads A) are detected and rejected.

- **File Existence**: The loaded file must exist at the resolved path; non-existent files will cause an error.

These semantics ensure secure, predictable loading without exposing scripts to external or unauthorized files. For best practices, organize scripts in clear directory structures and use `@` for reusable vendored modules.