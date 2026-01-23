

## File.digest

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">digest</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/execution_log/digest">digest</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

File digest. Always omitted for unresolved symlinks. May be omitted for empty files.

***

## File.is\_tool

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">is_tool</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Whether the file is a tool. Only set for inputs, never for outputs.

***

## File.path

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">path</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Path to the file relative to the execution root.

***

## File.symlink\_target\_path

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">symlink_target_path</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Symlink target path. Only set for unresolved symlinks.
