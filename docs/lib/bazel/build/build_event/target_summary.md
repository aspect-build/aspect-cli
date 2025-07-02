

## TargetSummary.overall\_build\_success

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">overall_build_success</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Conjunction of TargetComplete events for this target, including aspects.

***

## TargetSummary.overall\_test\_status

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">overall_test_status</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

For non-test targets, set to NO\_STATUS. For test targets, identical to TestSummary.overall\_status.

There are some cases where the `build` command on a test target succeeds,
but the `test` command on the same target results in FAILED\_TO\_BUILD. In
such cases, TargetComplete.overall\_build\_success is true, but this field is
FAILED\_TO\_BUILD, and TestSummary may be missing.
TODO - b/186996003: TestSummary is a child of TargetComplete and should be
posted.
