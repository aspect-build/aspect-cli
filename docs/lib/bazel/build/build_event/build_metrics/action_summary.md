

## ActionSummary.action\_cache\_statistics

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">action_cache_statistics</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">action_cache_statistics</span></span></span></code></pre>

***

## ActionSummary.action\_data

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">action_data</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/build_metrics/action_summary/action_data">action_data</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Contains the top N actions by number of actions executed.

***

## ActionSummary.actions\_created

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">actions_created</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The total number of actions created and registered during the build, including both aspects and configured targets. This metric includes unused actions that were constructed but not executed during this build. It does not include actions that were created on prior builds that are still valid, even if those actions had to be re-executed on this build. For the total number of actions that would be created if this invocation were "clean", see BuildGraphMetrics below.

***

## ActionSummary.actions\_created\_not\_including\_aspects

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">actions_created_not_including_aspects</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The total number of actions created this build just by configured targets. Used mainly to allow consumers of actions\_created, which used to not include aspects' actions, to normalize across the Blaze release that switched actions\_created to include all created actions.

***

## ActionSummary.actions\_executed

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">actions_executed</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The total number of actions executed during the build. This includes any remote cache hits, but excludes local action cache hits.

***

## ActionSummary.runner\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ActionSummary</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">runner_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/build_metrics/action_summary/runner_count">runner_count</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>
