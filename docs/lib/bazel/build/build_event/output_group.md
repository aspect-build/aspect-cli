

`property` **OutputGroup.file\_sets**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">OutputGroup</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">file_sets</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">named_set_of_files_id</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

List of file sets that belong to this output group as well.

`property` **OutputGroup.incomplete**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">OutputGroup</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">incomplete</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

Indicates that one or more of the output group's files were not built successfully (the generating action failed).

`property` **OutputGroup.inline\_files**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">OutputGroup</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">inline_files</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/bazel/build/build_event/file">file</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Inlined files that belong to this output group, requested via --build\_event\_inline\_output\_groups.

`property` **OutputGroup.name**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">OutputGroup</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">name</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

Name of the output group
