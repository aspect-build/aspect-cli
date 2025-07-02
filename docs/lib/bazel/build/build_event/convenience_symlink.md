

## ConvenienceSymlink.action

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ConvenienceSymlink</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">action</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The operation we are performing on the symlink.

***

## ConvenienceSymlink.path

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ConvenienceSymlink</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">path</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The path of the symlink to be created or deleted, absolute or relative to the workspace, creating any directories necessary. If a symlink already exists at that location, then it should be replaced by a symlink pointing to the new target.

***

## ConvenienceSymlink.target

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ConvenienceSymlink</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">target</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

If action is CREATE, this is the target path (relative to the output base) that the symlink should point to.

If action is DELETE, this field is not set.
