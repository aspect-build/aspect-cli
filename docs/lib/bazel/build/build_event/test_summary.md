

`property` **TestSummary.attempt\_count**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">attempt_count</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

Number of attempts. If there are a different number of attempts per shard, the highest attempt count across all shards for each run is used.

`property` **TestSummary.failed**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">failed</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/bazel/build/build_event/file">file</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Path to logs of failed runs;

`property` **TestSummary.overall\_status**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">overall_status</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

Wrapper around BlazeTestStatus to support importing that enum to proto3. Overall status of test, accumulated over all runs, shards, and attempts.

`property` **TestSummary.passed**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">passed</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/bazel/build/build_event/file">file</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Path to logs of passed runs.

`property` **TestSummary.run\_count**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">run_count</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

Value of runs\_per\_test for the test.

`property` **TestSummary.shard\_count**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">shard_count</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

Number of shards.

`property` **TestSummary.total\_num\_cached**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">total_num_cached</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

Total number of cached test actions

`property` **TestSummary.total\_run\_count**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">total_run_count</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

Total number of shard attempts. E.g., if a target has 4 runs, 3 shards, each with 2 attempts, then total\_run\_count will be 4*3*2 = 24.
