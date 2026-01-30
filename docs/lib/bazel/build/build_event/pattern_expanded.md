

`type` [TestSuiteExpansion](/lib/bazel/build/build_event/pattern_expanded/test_suite_expansion)

`property` **PatternExpanded.test\_suite\_expansions**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">PatternExpanded</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">test_suite_expansions</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">test_suite_expansion</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

All test suites requested via top-level target patterns. Does not include test suites whose label matched a negative pattern.
