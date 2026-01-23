

`property` **SymlinkEntrySet.direct\_entries**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SymlinkEntrySet</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">direct_entries</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/dict">dict</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a>, <a href="/lib/int">int</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

A map from relative paths of runfiles symlinks to the entry IDs of the symlink target, which may be a file, directory, or unresolved symlink.

`property` **SymlinkEntrySet.transitive\_set\_ids**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SymlinkEntrySet</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">transitive_set_ids</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/int">int</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Entry IDs of other symlink entry sets transitively contained in this set.
