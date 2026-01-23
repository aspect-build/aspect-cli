

`property` **File.digest**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">digest</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">digest</span></span></span></code></pre>

File digest. Always omitted for unresolved symlinks. May be omitted for empty files.

`property` **File.is\_tool**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">is_tool</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

Whether the file is a tool. Only set for inputs, never for outputs.

`property` **File.path**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">path</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

Path to the file relative to the execution root.

`property` **File.symlink\_target\_path**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">symlink_target_path</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

Symlink target path. Only set for unresolved symlinks.
