

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">bool</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">x</span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[bool](https://github.com/bazelbuild/starlark/blob/master/spec.md#bool): returns the truth value of any starlark value.

```
bool() == False
bool([]) == False
bool([1]) == True
bool(True) == True
bool(False) == False
bool(None) == False
bool(bool) == True
bool(1) == True
bool(0) == False
bool({}) == False
bool({1:2}) == True
bool(()) == False
bool((1,)) == True
bool("") == False
bool("1") == True
```
