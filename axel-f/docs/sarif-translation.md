# SARIF to GitHub PR Comments Translation

This document describes how to translate SARIF (Static Analysis Results Interchange Format) output from linters into GitHub PR review comments.

## SARIF Input (from linter)

```json
{
  "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
  "runs": [
    {
      "tool": {
        "driver": {
          "name": "ESLint"
        }
      },
      "results": [
        {
          "level": "error",
          "message": {
            "text": "'foo' is defined but never used"
          },
          "locations": [
            {
              "physicalLocation": {
                "artifactLocation": {
                  "uri": "src/index.ts"
                },
                "region": {
                  "startLine": 5,
                  "startColumn": 7,
                  "endLine": 5,
                  "endColumn": 10
                }
              }
            }
          ]
        }
      ]
    }
  ]
}
```

## GitHub PR Review API

**Endpoint:** `POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews`

**Payload:**
```json
{
  "commit_id": "abc123def456",
  "event": "COMMENT",
  "body": "Lint results",
  "comments": [
    {
      "path": "src/index.ts",
      "line": 5,
      "side": "RIGHT",
      "body": "**ESLint** (error)\n\n'foo' is defined but never used"
    }
  ]
}
```

---

## Field Mapping: SARIF to GitHub

| SARIF Path | GitHub Field |
|------------|--------------|
| `runs[].tool.driver.name` | Used in `comments[].body` prefix |
| `runs[].results[].level` | Used in `comments[].body` (error/warning) |
| `runs[].results[].message.text` | `comments[].body` content |
| `runs[].results[].locations[].physicalLocation.artifactLocation.uri` | `comments[].path` |
| `runs[].results[].locations[].physicalLocation.region.startLine` | `comments[].line` (single-line) or `comments[].start_line` (multi-line) |
| `runs[].results[].locations[].physicalLocation.region.endLine` | `comments[].line` (end of range) |

---

## Patch Input (unified diff)

```diff
--- a/src/index.ts
+++ b/src/index.ts
@@ -8,4 +8,3 @@
 const bar = 1;
-const unused = 2;
-const baz = unused + 1;
+const baz = bar + 1;
```

## GitHub PR Review with Suggestion

```json
{
  "commit_id": "abc123def456",
  "event": "COMMENT",
  "comments": [
    {
      "path": "src/index.ts",
      "start_line": 9,
      "line": 10,
      "side": "RIGHT",
      "body": "**ESLint**\n\n```suggestion\nconst baz = bar + 1;\n```"
    }
  ]
}
```

---

## Field Mapping: Patch to GitHub

| Patch Component | GitHub Field |
|-----------------|--------------|
| `+++ b/path/to/file` (strip `b/`) | `comments[].path` |
| `@@ -8,4 +8,3 @@` new start line | `comments[].start_line` |
| Last affected line in hunk | `comments[].line` |
| Lines starting with `+` (without the `+`) | Content inside ` ```suggestion ``` ` block |

---

## Multi-line Example

**SARIF with range:**
```json
{
  "runs": [{
    "tool": {"driver": {"name": "Ruff"}},
    "results": [{
      "level": "warning",
      "message": {"text": "Multiple imports on one line"},
      "locations": [{
        "physicalLocation": {
          "artifactLocation": {"uri": "src/main.py"},
          "region": {
            "startLine": 1,
            "endLine": 3
          }
        }
      }]
    }]
  }]
}
```

**GitHub comment (multi-line):**
```json
{
  "commit_id": "abc123",
  "event": "COMMENT",
  "comments": [{
    "path": "src/main.py",
    "start_line": 1,
    "line": 3,
    "side": "RIGHT",
    "body": "**Ruff** (warning)\n\nMultiple imports on one line"
  }]
}
```

---

## GitHub Suggestion Syntax

The `body` field uses GitHub's suggestion markdown:

```markdown
**ToolName**

```suggestion
replacement code here
line 2 of replacement
```
```

When rendered, GitHub displays an "Apply suggestion" button allowing one-click fixes.
