

`property` **CumulativeMetrics.num\_analyses**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">CumulativeMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">num_analyses</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

One-indexed number of "analyses" the server has run, including the current one. Will be incremented for every build/test/cquery/etc. command that reaches the analysis phase.

`property` **CumulativeMetrics.num\_builds**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">CumulativeMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">num_builds</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

One-indexed number of "builds" the server has run, including the current one. Will be incremented for every build/test/run/etc. command that reaches the execution phase.
