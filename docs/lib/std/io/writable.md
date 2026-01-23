

## Writable.close

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Writable</span></span>.<span class="entity name function python"><span class="meta generic-name python">close</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Closes this output stream. This drops the underlying handle, and subsequent writes will fail with "stream is closed".

***

## Writable.flush

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Writable</span></span>.<span class="entity name function python"><span class="meta generic-name python">flush</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Flushes this output stream, ensuring that all intermediately buffered contents reach their destination.

***

## Writable.is\_tty

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Writable</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">is_tty</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Returns true if the underlying stream is connected to a terminal/tty.

***

## Writable.write

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Writable</span></span>.<span class="entity name function python"><span class="meta generic-name python">write</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">buf</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Writes a buffer into this writer, returning how many bytes were written.

This function will attempt to write the entire contents of `buf`, but
the entire write may not succeed, or the write may also generate an
error. Typically data is written from the start of the buffer and then
caller should call `flush` to finish the write operation.
