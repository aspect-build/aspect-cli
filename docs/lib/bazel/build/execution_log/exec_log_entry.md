

`type` [Directory](/lib/bazel/build/execution_log/exec_log_entry/directory)

`type` [Output](/lib/std/process/output)

`type` [File](/lib/bazel/build/execution_log/exec_log_entry/file)

`type` [Spawn](/lib/bazel/build/execution_log/exec_log_entry/spawn)

`type` [SymlinkAction](/lib/bazel/build/execution_log/exec_log_entry/symlink_action)

`type` [UnresolvedSymlink](/lib/bazel/build/execution_log/exec_log_entry/unresolved_symlink)

`type` [RunfilesTree](/lib/bazel/build/execution_log/exec_log_entry/runfiles_tree)

`type` [Invocation](/lib/bazel/build/execution_log/exec_log_entry/invocation)

`type` [SymlinkEntrySet](/lib/bazel/build/execution_log/exec_log_entry/symlink_entry_set)

`type` [InputSet](/lib/bazel/build/execution_log/exec_log_entry/input_set)

`module` [output](/lib/bazel/build/execution_log/exec_log_entry/output)

`property` **ExecLogEntry.id**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecLogEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">id</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

If nonzero, then this entry may be referenced by later entries by this ID. Nonzero IDs are unique within an execution log, but may not be contiguous.

`property` **ExecLogEntry.type**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecLogEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">type</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">directory</span></span> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/build_event/file">file</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">input_set</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">invocation</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">runfiles_tree</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">spawn</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">symlink_action</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">symlink_entry_set</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">unresolved_symlink</span></span></span></code></pre>

The entry payload.
