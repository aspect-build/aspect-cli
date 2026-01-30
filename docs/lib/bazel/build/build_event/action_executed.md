

`property` **ActionExecuted.action\_metadata\_logs**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionExecuted</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">action_metadata_logs</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/bazel/build/build_event/file">file</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

List of paths to log files

`property` **ActionExecuted.command\_line**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionExecuted</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">command_line</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The command-line of the action, if the action is a command.

`property` **ActionExecuted.exit\_code**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionExecuted</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">exit_code</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

The exit code of the action, if it is available.

`property` **ActionExecuted.failure\_detail**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionExecuted</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">failure_detail</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">failure_detail</span></span></span></code></pre>

Only populated if success = false, and sometimes not even then.

`property` **ActionExecuted.primary\_output**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionExecuted</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">primary_output</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/build_event/file">file</a></span></code></pre>

Primary output; only provided for successful actions.

`property` **ActionExecuted.stderr**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionExecuted</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">stderr</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/build_event/file">file</a></span></code></pre>

Location where to find the standard error of the action (e.g., a file path).

`property` **ActionExecuted.stdout**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionExecuted</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">stdout</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/build_event/file">file</a></span></code></pre>

Location where to find the standard output of the action (e.g., a file path).

`property` **ActionExecuted.success**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionExecuted</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">success</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

`property` **ActionExecuted.type**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionExecuted</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">type</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The mnemonic of the action that was executed
