

`property` **SelectorEntry.boolean\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">boolean_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

`property` **SelectorEntry.fileset\_list\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">fileset_list_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/bazel/query/fileset_entry">fileset_entry</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

`property` **SelectorEntry.int\_list\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">int_list_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/int">int</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

`property` **SelectorEntry.int\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">int_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/int">int</a></span></code></pre>

Exactly one of the following fields (except for glob\_criteria) must be populated - note that the BOOLEAN and TRISTATE caveat in Attribute's comment does not apply here. The type field in the SelectorList containing this entry indicates which of these fields is populated, in accordance with the comments on Discriminator enum values above. (To be explicit: BOOLEAN populates the boolean\_value field and TRISTATE populates the tristate\_value field.)

`property` **SelectorEntry.is\_default\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">is_default_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

True if the entry's value is the default value for the type as a result of the condition value being specified as None (ie: {"//condition": None}).

`property` **SelectorEntry.label**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">label</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></code></pre>

The key of the selector entry. At this time, this is the label of a config\_setting rule, or the pseudo-label "//conditions:default".

`property` **SelectorEntry.label\_dict\_unary\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">label_dict_unary_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">label_dict_unary_entry</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

`property` **SelectorEntry.label\_keyed\_string\_dict\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">label_keyed_string_dict_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">label_keyed_string_dict_entry</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

`property` **SelectorEntry.label\_list\_dict\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">label_list_dict_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">label_list_dict_entry</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

`property` **SelectorEntry.license**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">license</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">license</span></span></span></code></pre>

`property` **SelectorEntry.string\_dict\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">string_dict_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">string_dict_entry</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

`property` **SelectorEntry.string\_list\_dict\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">string_list_dict_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">string_list_dict_entry</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

`property` **SelectorEntry.string\_list\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">string_list_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

`property` **SelectorEntry.string\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">string_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></code></pre>

`property` **SelectorEntry.tristate\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorEntry</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">tristate_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/int">int</a></span></code></pre>
