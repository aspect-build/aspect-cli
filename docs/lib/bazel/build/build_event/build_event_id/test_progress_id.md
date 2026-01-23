

## TestProgressId.attempt

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestProgressId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">attempt</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The execution attempt number which may increase due to retries (e.g. for flaky tests).

***

## TestProgressId.configuration

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestProgressId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">configuration</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/build_event_id/configuration_id">configuration_id</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The configuration under which the action is running.

***

## TestProgressId.label

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestProgressId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">label</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The label of the target for the action.

***

## TestProgressId.opaque\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestProgressId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">opaque_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

An incrementing count used to differentiate TestProgressIds for the same test attempt.

***

## TestProgressId.run

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestProgressId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">run</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The run number of the test action (e.g. for runs\_per\_test > 1).

***

## TestProgressId.shard

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestProgressId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">shard</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

For sharded tests, the shard number of the test action.
