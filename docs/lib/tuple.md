

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">tuple</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">a</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta qualified-name python"><span class="meta generic-name python">typing</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Iterable</span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta qualified-name python"><span class="support type python">tuple</span></span></span></span></code></pre>

[tuple](https://github.com/bazelbuild/starlark/blob/master/spec.md#tuple): returns a tuple containing the elements of the iterable x.

With no arguments, `tuple()` returns the empty tuple.

```
tuple() == ()
tuple([1,2,3]) == (1, 2, 3)
```
