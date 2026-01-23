

`property` **TestProgressId.attempt**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestProgressId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">attempt</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

The execution attempt number which may increase due to retries (e.g. for flaky tests).

`property` **TestProgressId.configuration**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestProgressId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">configuration</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">configuration_id</span></span></span></code></pre>

The configuration under which the action is running.

`property` **TestProgressId.label**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestProgressId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">label</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The label of the target for the action.

`property` **TestProgressId.opaque\_count**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestProgressId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">opaque_count</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

An incrementing count used to differentiate TestProgressIds for the same test attempt.

`property` **TestProgressId.run**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestProgressId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">run</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

The run number of the test action (e.g. for runs\_per\_test > 1).

`property` **TestProgressId.shard**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestProgressId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">shard</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

For sharded tests, the shard number of the test action.
