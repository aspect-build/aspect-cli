

`property` **FilesetEntry.destination\_directory**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">FilesetEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">destination_directory</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The relative path within the fileset rule where files will be mapped.

`property` **FilesetEntry.exclude**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">FilesetEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">exclude</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

If this is a fileset entry representing files within the rule package, this lists relative paths to files that should be excluded from the set.  This cannot contain values if 'file' also has values.

`property` **FilesetEntry.file**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">FilesetEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">file</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

A list of file labels to include from the source directory.

`property` **FilesetEntry.files\_present**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">FilesetEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">files_present</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

Whether the files= attribute was specified. This is necessary because no files= attribute and files=\[] mean different things.

`property` **FilesetEntry.source**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">FilesetEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">source</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The label pointing to the source target where files are copied from.

`property` **FilesetEntry.strip\_prefix**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">FilesetEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">strip_prefix</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></code></pre>

The prefix to strip from the path of the files in this FilesetEntry. Note that no value and the empty string as the value mean different things here.

`property` **FilesetEntry.symlink\_behavior**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">FilesetEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">symlink_behavior</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/int">int</a></span></code></pre>

This field is optional because there will be some time when the new PB is used by tools depending on blaze query, but the new blaze version is not yet released. TODO(bazel-team): Make this field required once a version of Blaze is released that outputs this field.
