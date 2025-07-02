

## Invocation.hash\_function\_name

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Invocation</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">hash_function_name</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The hash function used to compute digests.

***

## Invocation.id

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Invocation</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">id</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The ID of the invocation.

***

## Invocation.sibling\_repository\_layout

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Invocation</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">sibling_repository_layout</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Whether --experimental\_sibling\_repository\_layout is enabled.

***

## Invocation.workspace\_runfiles\_directory

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Invocation</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">workspace_runfiles_directory</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The name of the subdirectory of the runfiles tree corresponding to the main repository (also known as the "workspace name").

With --enable\_bzlmod, this is always "\_main", but can vary when using
WORKSPACE.
