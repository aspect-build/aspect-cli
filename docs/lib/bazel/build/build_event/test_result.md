

## TestResult.cached\_locally

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestResult</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">cached_locally</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

True, if the reported attempt is taken from the tool's local cache.

***

## TestResult.execution\_info

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestResult</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">execution_info</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/test_result/execution_info">execution_info</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

***

## TestResult.status

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestResult</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">status</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The status of this test.

***

## TestResult.status\_details

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestResult</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">status_details</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Additional details about the status of the test. This is intended for user display and must not be parsed.

***

## TestResult.test\_action\_output

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestResult</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">test_action_output</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/execution_log/exec_log_entry/file">file</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Files (logs, test.xml, undeclared outputs, etc) generated by that test action.

***

## TestResult.warning

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TestResult</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">warning</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Warnings generated by that test action.
