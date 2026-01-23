

`function` **Readable.read**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Readable</span></span>.<span class="entity name function python"><span class="meta generic-name python">read</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">size</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/int">int</a> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta qualified-name python"><span class="meta generic-name python">Bytes</span></span></span></span></code></pre>

Reads bytes from this source.

If `size` is provided, reads up to that many bytes.
If `size` is not provided, reads until EOF.
Returns the bytes read.

`function` **Readable.read\_to\_string**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Readable</span></span>.<span class="entity name function python"><span class="meta generic-name python">read_to_string</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/str">str</a></span></span></code></pre>

Reads all bytes until EOF in this source and returns a string.

If successful, this function will return all bytes as a string.

`property` **Readable.is\_tty**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Readable</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">is_tty</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

Returns true if the underlying stream is connected to a terminal/tty.
