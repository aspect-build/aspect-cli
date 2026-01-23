

`property` **AttributeDefinition.allow\_empty**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AttributeDefinition</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">allow_empty</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

type=*\_list|*\_dict

`property` **AttributeDefinition.allow\_single\_file**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AttributeDefinition</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">allow_single_file</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

type=label

`property` **AttributeDefinition.allowed\_rule\_classes**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AttributeDefinition</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">allowed_rule_classes</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/query/allowed_rule_class_info">allowed_rule_class_info</a></span></code></pre>

type=label\*

`property` **AttributeDefinition.cfg\_is\_host**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AttributeDefinition</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">cfg_is_host</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

edge entails a transition to "host" configuration

`property` **AttributeDefinition.configurable**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AttributeDefinition</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">configurable</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

`property` **AttributeDefinition.default**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AttributeDefinition</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">default</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/query/attribute_value">attribute_value</a></span></code></pre>

simple (not computed/late-bound) values only

`property` **AttributeDefinition.documentation**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AttributeDefinition</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">documentation</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></code></pre>

`property` **AttributeDefinition.executable**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AttributeDefinition</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">executable</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

type=label

`property` **AttributeDefinition.mandatory**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AttributeDefinition</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">mandatory</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

`property` **AttributeDefinition.name**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AttributeDefinition</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">name</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

e.g. "name", "srcs"

`property` **AttributeDefinition.nodep**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AttributeDefinition</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">nodep</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

label-valued edge does not establish a dependency

`property` **AttributeDefinition.type**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AttributeDefinition</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">type</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>
