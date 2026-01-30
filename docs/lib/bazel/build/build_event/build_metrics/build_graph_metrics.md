

`type` [RuleClassCount](/lib/bazel/build/build_event/build_metrics/build_graph_metrics/rule_class_count)

`type` [AspectCount](/lib/bazel/build/build_event/build_metrics/build_graph_metrics/aspect_count)

`property` **BuildGraphMetrics.action\_count**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">action_count</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

How many actions belonged to the configured targets/aspects above. It may not be necessary to execute all of these actions to build the requested targets. May not be populated if analysis phase was fully cached.

`property` **BuildGraphMetrics.action\_count\_not\_including\_aspects**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">action_count_not_including_aspects</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

How many actions belonged to configured targets: always at most action\_count. Useful mainly for historical comparisons to ActionMetrics.actions\_created, which used to not count aspects' actions.

`property` **BuildGraphMetrics.action\_lookup\_value\_count**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">action_lookup_value_count</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

How many configured targets/aspects were in this build, including any that were analyzed on a prior build and are still valid. May not be populated if analysis phase was fully cached. Note: for historical reasons this includes input/output files and other configured targets that do not actually have associated actions.

`property` **BuildGraphMetrics.action\_lookup\_value\_count\_not\_including\_aspects**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">action_lookup_value_count_not_including_aspects</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

How many configured targets alone were in this build: always at most action\_lookup\_value\_count. Useful mainly for historical comparisons to TargetMetrics.targets\_configured, which used to not count aspects. This also includes configured targets that do not have associated actions.

`property` **BuildGraphMetrics.aspect**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">aspect</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">aspect_count</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

`property` **BuildGraphMetrics.built\_values**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">built_values</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">evaluation_stat</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Number of SkyValues that were built. This means that they were evaluated and were found to have changed from their previous version.

`property` **BuildGraphMetrics.changed\_values**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">changed_values</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">evaluation_stat</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Number of SkyValues that changed by themselves. For example, when a file on the file system changes, the SkyValue representing it will change.

`property` **BuildGraphMetrics.cleaned\_values**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">cleaned_values</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">evaluation_stat</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Number of SkyValues that were evaluated and found clean, i.e. equal to their previous version.

`property` **BuildGraphMetrics.dirtied\_values**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">dirtied_values</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">evaluation_stat</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Number of SkyValues that were dirtied during the build. Dirtied nodes are those that transitively depend on a node that changed by itself (e.g. one representing a file in the file system)

`property` **BuildGraphMetrics.evaluated\_values**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">evaluated_values</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">evaluation_stat</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Number of evaluations to build SkyValues. This includes restarted evaluations, which means there can be multiple evaluations per built SkyValue. Subtract built\_values from this number to get the number of restarted evaluations.

`property` **BuildGraphMetrics.input\_file\_configured\_target\_count**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">input_file_configured_target_count</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

How many "input file" configured targets there were: one per source file. Should agree with artifact\_metrics.source\_artifacts\_read.count above,

`property` **BuildGraphMetrics.other\_configured\_target\_count**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">other_configured_target_count</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

How many "other" configured targets there were (like alias, package\_group, and other non-rule non-file configured targets).

`property` **BuildGraphMetrics.output\_artifact\_count**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">output_artifact_count</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

How many artifacts are outputs of the above actions. May not be populated if analysis phase was fully cached.

`property` **BuildGraphMetrics.output\_file\_configured\_target\_count**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">output_file_configured_target_count</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

How many "output file" configured targets there were: output files that are targets (not implicit outputs).

`property` **BuildGraphMetrics.post\_invocation\_skyframe\_node\_count**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">post_invocation_skyframe_node_count</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

How many Skyframe nodes there are in memory at the end of the build. This may underestimate the number of nodes when running with memory-saving settings or with Skybuild, and may overestimate if there are nodes from prior evaluations still in the cache.

`property` **BuildGraphMetrics.rule\_class**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildGraphMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">rule_class</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">rule_class_count</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>
