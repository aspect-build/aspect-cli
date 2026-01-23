

`property` **TargetComplete.directory\_output**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetComplete</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">directory_output</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/bazel/build/build_event/file">file</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Report output artifacts (referenced transitively via output\_group) which emit directories instead of singleton files. These directory\_output entries will never include a uri.

`property` **TargetComplete.failure\_detail**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetComplete</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">failure_detail</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">failure_detail</span></span></span></code></pre>

Failure information about the target, only populated if success is false, and sometimes not even then. Equal to one of the ActionExecuted failure\_detail fields for one of the root cause ActionExecuted events.

`property` **TargetComplete.output\_group**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetComplete</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">output_group</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">output_group</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The output files are arranged by their output group. If an output file is part of multiple output groups, it appears once in each output group.

`property` **TargetComplete.success**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetComplete</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">success</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

`property` **TargetComplete.tag**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetComplete</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">tag</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

List of tags associated with this configured target.
