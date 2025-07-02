

## Progress.stderr

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Progress</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">stderr</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The next chunk of stderr that bazel produced since the last progress event or the beginning of the build. Consumers that need to reason about the relative order of stdout and stderr can assume that stderr has been emitted before stdout if both are present, on a best-effort basis.

***

## Progress.stdout

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Progress</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">stdout</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The next chunk of stdout that bazel produced since the last progress event or the beginning of the build. Consumers that need to reason about the relative order of stdout and stderr can assume that stderr has been emitted before stdout if both are present, on a best-effort basis.
