

## WorkerMetrics.actions\_executed

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">actions_executed</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

***

## WorkerMetrics.code

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">code</span></span><span class="punctuation separator annotation variable python">:</span> <span class="constant language python">None</span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

***

## WorkerMetrics.is\_measurable

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">is_measurable</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

TODO(b/300067854): Deprecate since all worker metrics should have their WorkerStats set.

***

## WorkerMetrics.is\_multiplex

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">is_multiplex</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Multiplex or singleplex worker.

***

## WorkerMetrics.is\_sandbox

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">is_sandbox</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Using worker sandbox file system or not.

***

## WorkerMetrics.mnemonic

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">mnemonic</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Mnemonic of running worker.

***

## WorkerMetrics.prior\_actions\_executed

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">prior_actions_executed</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

***

## WorkerMetrics.process\_id

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">process_id</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Worker process id. If there is no process for worker, equals to zero.

***

## WorkerMetrics.worker\_ids

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">worker_ids</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">list</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Ids of workers. Could be multiple in case of multiplex workers

***

## WorkerMetrics.worker\_key\_hash

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">worker_key_hash</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Hash value of worker key. Needed to distinguish worker pools with same menmonic but with different worker keys.

***

## WorkerMetrics.worker\_stats

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">worker_stats</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">list</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/build_metrics/worker_metrics/worker_stats">worker_stats</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Combined workers statistics.

***

## WorkerMetrics.worker\_status

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">worker_status</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>
