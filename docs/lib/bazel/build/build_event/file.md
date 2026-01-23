

## File.digest

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">digest</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Digest of the file, using the build tool's configured digest algorithm, hex-encoded.

***

## File.file

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">file</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

***

## File.length

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">length</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Length of the file in bytes.

***

## File.name

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">name</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

identifier indicating the nature of the file (e.g., "stdout", "stderr")

***

## File.path\_prefix

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">File</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">path_prefix</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

A sequence of prefixes to apply to the file name to construct a full path. In most but not all cases, there will be 3 entries:

1. A root output directory, eg "bazel-out"
2. A configuration mnemonic, eg "k8-fastbuild"
3. An output category, eg "genfiles"
