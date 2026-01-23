

`type` [Selector](/lib/bazel/query/attribute/selector)

`type` [SelectorEntry](/lib/bazel/query/attribute/selector_entry)

`type` [SelectorList](/lib/bazel/query/attribute/selector_list)

`property` **Attribute.boolean\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">boolean_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

If the attribute has a boolean value this will be populated.

`property` **Attribute.explicitly\_specified**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">explicitly_specified</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

Whether the attribute was explicitly specified

`property` **Attribute.fileset\_list\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">fileset_list_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/bazel/query/fileset_entry">fileset_entry</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

If the attribute is part of a Fileset, the fileset entries are stored in this field.

`property` **Attribute.int\_list\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">int_list_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/int">int</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The value of the attribute has a list of int32 values

`property` **Attribute.int\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">int_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/int">int</a></span></code></pre>

If this attribute has an integer value this will be populated. Boolean and TriState also use this field as \[0,1] and \[-1,0,1] for \[false, true] and \[auto, no, yes] respectively.

`property` **Attribute.label\_dict\_unary\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">label_dict_unary_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">label_dict_unary_entry</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

If this is a label dict unary, each entry will be stored here.

`property` **Attribute.label\_keyed\_string\_dict\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">label_keyed_string_dict_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">label_keyed_string_dict_entry</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

If this is a label-keyed string dict, each entry will be stored here.

`property` **Attribute.label\_list\_dict\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">label_list_dict_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">label_list_dict_entry</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

If this is a label list dict, each entry will be stored here.

`property` **Attribute.license**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">license</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">license</span></span></span></code></pre>

If this is a license attribute, the license information is stored here.

`property` **Attribute.name**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">name</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The name of the attribute

`property` **Attribute.nodep**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">nodep</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

If this attribute has a string value or a string list value, then this may be set to indicate that the value may be treated as a label that isn't a dependency of this attribute's rule.

`property` **Attribute.selector\_list**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">selector_list</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">selector_list</span></span></span></code></pre>

If this attribute's value is an expression containing one or more select expressions, then its type is SELECTOR\_LIST and a SelectorList will be stored here.

`property` **Attribute.source\_aspect\_name**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">source_aspect_name</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></code></pre>

Represents the aspect that this attribute comes from. It is set to an empty string if it does not come from an aspect.

`property` **Attribute.string\_dict\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">string_dict_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">string_dict_entry</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

If this is a string dict, each entry will be stored here.

`property` **Attribute.string\_list\_dict\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">string_list_dict_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">string_list_dict_entry</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

If this is a string list dict, each entry will be stored here.

`property` **Attribute.string\_list\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">string_list_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The value of the attribute has a list of string values (label and path note from STRING applies here as well).

`property` **Attribute.string\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">string_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></code></pre>

If the attribute has a string value this will be populated.  Label and path attributes use this field as the value even though the type may be LABEL or something else other than STRING.

`property` **Attribute.tristate\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">tristate_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/int">int</a></span></code></pre>

If the attribute is a Tristate value, this will be populated.

`property` **Attribute.type**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Attribute</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">type</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The type of attribute.  This message is used for all of the different attribute types so the discriminator helps for figuring out what is stored in the message.
