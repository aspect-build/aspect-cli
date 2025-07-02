

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">int</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">a</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/float">float</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">base</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[int](https://github.com/bazelbuild/starlark/blob/master/spec.md#int): convert a value to integer.

`int(x[, base])` interprets its argument as an integer.

If x is an `int`, the result is x.
If x is a `float`, the result is the integer value nearest to x,
truncating towards zero; it is an error if x is not finite (`NaN`,
`+Inf`, `-Inf`).
If x is a `bool`, the result is 0 for `False` or 1 for `True`.

If x is a string, it is interpreted like a string literal;
an optional base prefix (`0`, `0b`, `0B`, `0x`, `0X`) determines which
base to use. The string may specify an arbitrarily large integer,
whereas true integer literals are restricted to 64 bits.
If a non-zero `base` argument is provided, the string is interpreted
in that base and no base prefix is permitted; the base argument may
specified by name.

`int()` with no arguments returns 0.

```
int() == 0
int(1) == 1
int(False) == 0
int(True) == 1
int('1') == 1
int('16') == 16
int('16', 10) == 16
int('16', 8) == 14
int('16', 16) == 22
int(0.0) == 0
int(3.14) == 3
int(-12345.6789) == -12345
int(2e9) == 2000000000
int("hello")   # error: Cannot parse
int(float("nan"))   # error: cannot be represented as exact integer
int(float("inf"))   # error: cannot be represented as exact integer
```
