

`property` **SelectorList.elements**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorList</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">elements</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">selector</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The list of selector elements in this selector list. At this time, this cannot be empty, i.e. a selector list is never empty.

`property` **SelectorList.type**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SelectorList</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">type</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/int">int</a></span></code></pre>

The type that this selector list evaluates to, and the type that each selector in the list evaluates to. At this time, this cannot be SELECTOR\_LIST, i.e. selector lists do not nest.
