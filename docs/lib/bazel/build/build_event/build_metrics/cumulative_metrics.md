

## CumulativeMetrics.num\_analyses

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">CumulativeMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">num_analyses</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

One-indexed number of "analyses" the server has run, including the current one. Will be incremented for every build/test/cquery/etc. command that reaches the analysis phase.

***

## CumulativeMetrics.num\_builds

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">CumulativeMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">num_builds</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

One-indexed number of "builds" the server has run, including the current one. Will be incremented for every build/test/run/etc. command that reaches the execution phase.
