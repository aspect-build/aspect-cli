

`type` [MemoryMetrics](/lib/bazel/build/build_event/build_metrics/memory_metrics)

`type` [NetworkMetrics](/lib/bazel/build/build_event/build_metrics/network_metrics)

`type` [ArtifactMetrics](/lib/bazel/build/build_event/build_metrics/artifact_metrics)

`type` [EvaluationStat](/lib/bazel/build/build_event/build_metrics/evaluation_stat)

`type` [WorkerMetrics](/lib/bazel/build/build_event/build_metrics/worker_metrics)

`type` [TimingMetrics](/lib/bazel/build/build_event/build_metrics/timing_metrics)

`type` [WorkerPoolMetrics](/lib/bazel/build/build_event/build_metrics/worker_pool_metrics)

`type` [TargetMetrics](/lib/bazel/build/build_event/build_metrics/target_metrics)

`type` [ActionSummary](/lib/bazel/build/build_event/build_metrics/action_summary)

`type` [CumulativeMetrics](/lib/bazel/build/build_event/build_metrics/cumulative_metrics)

`type` [PackageMetrics](/lib/bazel/build/build_event/build_metrics/package_metrics)

`type` [BuildGraphMetrics](/lib/bazel/build/build_event/build_metrics/build_graph_metrics)

`type` [DynamicExecutionMetrics](/lib/bazel/build/build_event/build_metrics/dynamic_execution_metrics)

`module` [action\_summary](/lib/bazel/build/build_event/build_metrics/action_summary)

`module` [memory\_metrics](/lib/bazel/build/build_event/build_metrics/memory_metrics)

`module` [artifact\_metrics](/lib/bazel/build/build_event/build_metrics/artifact_metrics)

`module` [build\_graph\_metrics](/lib/bazel/build/build_event/build_metrics/build_graph_metrics)

`module` [worker\_metrics](/lib/bazel/build/build_event/build_metrics/worker_metrics)

`module` [network\_metrics](/lib/bazel/build/build_event/build_metrics/network_metrics)

`module` [worker\_pool\_metrics](/lib/bazel/build/build_event/build_metrics/worker_pool_metrics)

`module` [dynamic\_execution\_metrics](/lib/bazel/build/build_event/build_metrics/dynamic_execution_metrics)

`property` **BuildMetrics.action\_summary**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">action_summary</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/build_event/build_metrics/action_summary">action_summary</a></span></code></pre>

`property` **BuildMetrics.artifact\_metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">artifact_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/build_event/build_metrics/artifact_metrics">artifact_metrics</a></span></code></pre>

`property` **BuildMetrics.build\_graph\_metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">build_graph_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/build_event/build_metrics/build_graph_metrics">build_graph_metrics</a></span></code></pre>

`property` **BuildMetrics.cumulative\_metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">cumulative_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">cumulative_metrics</span></span></span></code></pre>

`property` **BuildMetrics.dynamic\_execution\_metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">dynamic_execution_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/build_event/build_metrics/dynamic_execution_metrics">dynamic_execution_metrics</a></span></code></pre>

`property` **BuildMetrics.memory\_metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">memory_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/build_event/build_metrics/memory_metrics">memory_metrics</a></span></code></pre>

`property` **BuildMetrics.network\_metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">network_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/build_event/build_metrics/network_metrics">network_metrics</a></span></code></pre>

`property` **BuildMetrics.package\_metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">package_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">package_metrics</span></span></span></code></pre>

`property` **BuildMetrics.target\_metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">target_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">target_metrics</span></span></span></code></pre>

`property` **BuildMetrics.timing\_metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">timing_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">timing_metrics</span></span></span></code></pre>

`property` **BuildMetrics.worker\_metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">worker_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/bazel/build/build_event/build_metrics/worker_metrics">worker_metrics</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

`property` **BuildMetrics.worker\_pool\_metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">worker_pool_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/build_event/build_metrics/worker_pool_metrics">worker_pool_metrics</a></span></code></pre>
