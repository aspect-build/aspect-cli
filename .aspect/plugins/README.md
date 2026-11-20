# Console plugins

BuildBuddy's CLI plugins, written as Aspect CLI features. They are examples of what Bazel's console stages (`bazel.output.*`) and the `BazelTrait` hooks can do. Each is registered in the root `MODULE.aspect` and configured in `.aspect/config.axl`.

| Feature | BuildBuddy plugin | Built from | On here |
|---|---|---|---|
| `Theme` ([theme.axl](theme.axl)) | theme-modern, theme-mono (`--theme:style`) | `rewrite` stages, on a terminal | yes, `modern` |
| `GoHighlight` ([go_highlight.axl](go_highlight.axl)) | go-highlight | `rewrite` stages | yes |
| `GoDeps` ([go_deps.axl](go_deps.axl)) | go-deps | an `observe` stage, and an `output_end` hook that runs `aspect gazelle` | yes, `ask` |
| `PingRemote` ([ping_remote.axl](ping_remote.axl)) | ping-remote | an `rc_flags` hook that connects to `--remote_executor` | yes |
| `Notify` ([notify.axl](notify.axl)) | notify | `build_event` hooks and a `post_task` hook | `--notify:enabled=true` |
| `OpenInvocation` ([open_invocation.axl](open_invocation.axl)) | open-invocation | `build_event` and `Phases.task_update` hooks and a `post_task` hook | `--open-invocation:enabled=true` |

Where they differ from the plugins:

- `Theme`: under a PTY, a rewritten row of Bazel's progress area is cut to the characters Bazel wrote, so the ⚡️ on a remote action's row costs it its last character. theme-mono's `([0-9+])s` typo, which left actions past 9 seconds unstyled, is fixed.
- `PingRemote` connects to the executor's port instead of sending an ICMP ping, which firewalls often drop. It checks `grpc://` and `grpcs://` executors only. Like the plugin, it clears `--remote_executor` alone, so an rc that also sets a `remote` strategy fails the build instead.
- `Notify` reads the build's result from the build event stream, not from Bazel's console lines.
- `OpenInvocation` also opens the page of a build whose events the CLI streams through its own sinks, keyed on their invocation id rather than Bazel's.
