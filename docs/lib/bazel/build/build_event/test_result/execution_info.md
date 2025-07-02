

## ExecutionInfo.cached\_remotely

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecutionInfo</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">cached_remotely</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

True, if the reported attempt was a cache hit in a remote cache.

***

## ExecutionInfo.exit\_code

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecutionInfo</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">exit_code</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The exit code of the test action.

***

## ExecutionInfo.hostname

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecutionInfo</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">hostname</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The hostname of the machine where the test action was executed (in case of remote execution), if known.

***

## ExecutionInfo.resource\_usage

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecutionInfo</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">resource_usage</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">list</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/test_result/execution_info/resource_usage">resource_usage</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

***

## ExecutionInfo.strategy

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecutionInfo</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">strategy</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Name of the strategy to execute this test action (e.g., "local", "remote")

***

## ExecutionInfo.timing\_breakdown

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecutionInfo</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">timing_breakdown</span></span><span class="punctuation separator annotation variable python">:</span> <span class="constant language python">None</span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/test_result/execution_info/timing_breakdown">timing_breakdown</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>
