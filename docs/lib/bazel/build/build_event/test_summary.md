

## TestSummary.attempt\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">attempt_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Number of attempts. If there are a different number of attempts per shard, the highest attempt count across all shards for each run is used.

***

## TestSummary.failed

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">failed</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/execution_log/exec_log_entry/file">file</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Path to logs of failed runs;

***

## TestSummary.overall\_status

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">overall_status</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Wrapper around BlazeTestStatus to support importing that enum to proto3. Overall status of test, accumulated over all runs, shards, and attempts.

***

## TestSummary.passed

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">passed</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/execution_log/exec_log_entry/file">file</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Path to logs of passed runs.

***

## TestSummary.run\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">run_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Value of runs\_per\_test for the test.

***

## TestSummary.shard\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">shard_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Number of shards.

***

## TestSummary.total\_num\_cached

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">total_num_cached</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Total number of cached test actions

***

## TestSummary.total\_run\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">total_run_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Total number of shard attempts. E.g., if a target has 4 runs, 3 shards, each with 2 attempts, then total\_run\_count will be 4*3*2 = 24.
