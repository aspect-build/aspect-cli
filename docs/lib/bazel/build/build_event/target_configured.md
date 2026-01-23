

`property` **TargetConfigured.tag**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetConfigured</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">tag</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

List of all tags associated with this target (for all possible configurations).

`property` **TargetConfigured.target\_kind**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetConfigured</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">target_kind</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The kind of target (e.g.,  e.g. "cc\_library rule", "source file", "generated file") where the completion is reported.

`property` **TargetConfigured.test\_size**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetConfigured</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">test_size</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The size of the test, if the target is a test target. Unset otherwise.
