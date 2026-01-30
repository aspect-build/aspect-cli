

`property` **ConfiguredRuleInput.configuration\_checksum**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ConfiguredRuleInput</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">configuration_checksum</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></code></pre>

Dep's configuration if the dep isn't a source file, else unset.

`property` **ConfiguredRuleInput.configuration\_id**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ConfiguredRuleInput</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">configuration_id</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/int">int</a></span></code></pre>

Reference to this dep's configuration if --proto:include\_configurations is set: see <https://github.com/bazelbuild/bazel/blob/7278be3f9b0c26842ecb8225f0215c1e4aede5a9/src/main/protobuf/analysis_v2.proto#L189.> If this dep is a source file, this is unset.

`property` **ConfiguredRuleInput.label**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ConfiguredRuleInput</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">label</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></code></pre>

Dep's target label.
