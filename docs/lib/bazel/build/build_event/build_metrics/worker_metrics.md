

`type` [WorkerStats](/lib/bazel/build/build_event/build_metrics/worker_metrics/worker_stats)

`property` **WorkerMetrics.actions\_executed**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">actions_executed</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

`property` **WorkerMetrics.code**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">code</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/int">int</a></span></code></pre>

`property` **WorkerMetrics.is\_measurable**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">is_measurable</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

TODO(b/300067854): Deprecate since all worker metrics should have their WorkerStats set.

`property` **WorkerMetrics.is\_multiplex**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">is_multiplex</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

Multiplex or singleplex worker.

`property` **WorkerMetrics.is\_sandbox**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">is_sandbox</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

Using worker sandbox file system or not.

`property` **WorkerMetrics.mnemonic**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">mnemonic</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

Mnemonic of running worker.

`property` **WorkerMetrics.prior\_actions\_executed**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">prior_actions_executed</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

`property` **WorkerMetrics.process\_id**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">process_id</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

Worker process id. If there is no process for worker, equals to zero.

`property` **WorkerMetrics.worker\_ids**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">worker_ids</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/int">int</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Ids of workers. Could be multiple in case of multiplex workers

`property` **WorkerMetrics.worker\_key\_hash**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">worker_key_hash</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

Hash value of worker key. Needed to distinguish worker pools with same menmonic but with different worker keys.

`property` **WorkerMetrics.worker\_stats**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">worker_stats</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">worker_stats</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Combined workers statistics.

`property` **WorkerMetrics.worker\_status**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">worker_status</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>
