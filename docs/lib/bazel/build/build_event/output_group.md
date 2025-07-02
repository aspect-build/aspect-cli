

## OutputGroup.file\_sets

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">OutputGroup</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">file_sets</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">list</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/build_event_id/named_set_of_files_id">named_set_of_files_id</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

List of file sets that belong to this output group as well.

***

## OutputGroup.incomplete

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">OutputGroup</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">incomplete</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Indicates that one or more of the output group's files were not built successfully (the generating action failed).

***

## OutputGroup.inline\_files

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">OutputGroup</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">inline_files</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">list</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/file">file</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Inlined files that belong to this output group, requested via --experimental\_build\_event\_output\_group\_mode.

***

## OutputGroup.name

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">OutputGroup</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">name</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Name of the output group
