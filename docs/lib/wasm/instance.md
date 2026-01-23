

`function` **Instance.get\_memory**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Instance</span></span>.<span class="entity name function python"><span class="meta generic-name python">get_memory</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">name</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/str">str</a></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/wasm/memory">Memory</a></span></span></code></pre>

`function` **Instance.list\_exports**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Instance</span></span>.<span class="entity name function python"><span class="meta generic-name python">list_exports</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

Returns a list of all exported names from the WASM module.

`function` **Instance.start**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Instance</span></span>.<span class="entity name function python"><span class="meta generic-name python">start</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/bool">bool</a></span></span></code></pre>

`property` **Instance.exports**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Instance</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">exports</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/wasm/exports">Exports</a></span></code></pre>
