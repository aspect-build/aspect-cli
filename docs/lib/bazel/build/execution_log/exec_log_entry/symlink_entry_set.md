

## SymlinkEntrySet.direct\_entries

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SymlinkEntrySet</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">direct_entries</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">dict</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span>, <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

A map from relative paths of runfiles symlinks to the entry IDs of the symlink target, which may be a file, directory, or unresolved symlink.

***

## SymlinkEntrySet.transitive\_set\_ids

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SymlinkEntrySet</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">transitive_set_ids</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">list</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Entry IDs of other symlink entry sets transitively contained in this set.
