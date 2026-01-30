

`property` **Selector.entries**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Selector</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">entries</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">selector_entry</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The list of (label, value) pairs in the map that defines the selector. At this time, this cannot be empty, i.e. a selector has at least one entry.

`property` **Selector.has\_default\_value**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Selector</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">has_default_value</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

Whether or not this has any default values.

`property` **Selector.no\_match\_error**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Selector</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">no_match_error</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></code></pre>

The error message when no condition matches.
