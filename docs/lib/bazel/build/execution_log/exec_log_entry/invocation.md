

`property` **Invocation.hash\_function\_name**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Invocation</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">hash_function_name</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The hash function used to compute digests.

`property` **Invocation.sibling\_repository\_layout**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Invocation</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">sibling_repository_layout</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

Whether --experimental\_sibling\_repository\_layout is enabled.

`property` **Invocation.workspace\_runfiles\_directory**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Invocation</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">workspace_runfiles_directory</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The name of the subdirectory of the runfiles tree corresponding to the main repository (also known as the "workspace name").

With --enable\_bzlmod, this is always "\_main", but can vary when using
WORKSPACE.
