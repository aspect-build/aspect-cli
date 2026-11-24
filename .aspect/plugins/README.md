# Example plugins

These are example plugins, ported from [BuildBuddy's open-source CLI plugins](https://www.buildbuddy.io/plugins/) to show what AXL can do to customize and extend Bazel's behavior. Each is an Aspect CLI feature built on Bazel's console stages (`bazel.output.*`) and the `BazelTrait` hooks, registered in the root `MODULE.aspect` and configured in `.aspect/config.axl`.

| Feature | Ported from | Built from | On here |
|---|---|---|---|
| `Theme` ([theme.axl](theme.axl)) | [theme-modern](https://github.com/siggisim/theme-modern), [theme-mono](https://github.com/siggisim/theme-mono) (`--theme:style`) | `rewrite` stages, on a terminal | yes, `modern` |
| `GoHighlight` ([go_highlight.axl](go_highlight.axl)) | [go-highlight](https://github.com/bduffany/go-highlight) | `rewrite` stages | yes |
| `GoDeps` ([go_deps.axl](go_deps.axl)) | [go-deps](https://github.com/buildbuddy-io/buildbuddy/tree/master/cli/plugins/go-deps) | an `observe` stage, and an `output_end` hook that runs `aspect gazelle` | yes, `ask` |
| `PingRemote` ([ping_remote.axl](ping_remote.axl)) | [ping-remote](https://github.com/siggisim/ping-remote) | an `rc_flags` hook that connects to `--remote_executor` | yes |
| `Notify` ([notify.axl](notify.axl)) | [notify](https://github.com/buildbuddy-io/buildbuddy/tree/master/cli/plugins/notify) | `build_event` hooks and a `post_task` hook | `--notify` |
| `OpenInvocation` ([open_invocation.axl](open_invocation.axl)) | [open-invocation](https://github.com/buildbuddy-io/buildbuddy/tree/master/cli/plugins/open-invocation) | `build_event` and `Phases.task_update` hooks and a `post_task` hook | `--open-invocation` |

## Size

Lines of code, not counting blank lines, comments or docstrings. A BuildBuddy plugin is a shell script, often handing off to a Python script, that reads Bazel's flags or output from a file. The AXL port is a single file that runs inside the CLI and includes its feature declaration and flags.

| Plugin | BuildBuddy | AXL |
|---|---|---|
| theme-modern + theme-mono | 102 (shell + Python, 53 + 49) | 60 (one feature, both styles) |
| go-highlight | 14 (shell + Python) | 23 (adds TypeScript locations) |
| go-deps | 149 (shell + Python) | 47 |
| ping-remote | 7 (shell) | 40 (any executor, not only BuildBuddy's) |
| notify | 45 (shell + Python) | 41 |
| open-invocation | 64 (shell + Python) | 43 (also CLI-streamed builds) |

## Differences from the originals

- `Theme`: under a PTY, a rewritten row of Bazel's progress area is cut to the terminal columns Bazel drew, so a running action's ⚡️ or 🔨 takes the place of three of its indent's spaces. theme-mono's `([0-9+])s` typo, which left actions past 9 seconds unstyled, is fixed.
- `GoHighlight` also rewrites TypeScript's `file.ts(line,col):` to `file.ts:line:col:`, so an editor's terminal can open it.
- `PingRemote` checks whatever executor the build uses, including one the CLI sets itself; the original acts only on a `*.buildbuddy.io` executor. It connects to the executor's port instead of sending an ICMP ping, which firewalls often drop. Like the original, it clears `--remote_executor` alone, so an rc that also sets a `remote` strategy fails the build instead.
- `Notify` reads the build's result from the build event stream, not from Bazel's console lines.
- `OpenInvocation` also opens the page of a build whose events the CLI streams through its own sinks, keyed on their invocation id rather than Bazel's.
