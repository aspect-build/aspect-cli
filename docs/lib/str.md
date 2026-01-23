

## str.capitalize

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">capitalize</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.capitalize](https://github.com/bazelbuild/starlark/blob/master/spec.md#string%C2%B7capitalize): returns a copy of string S, where the first character (if any) is converted to uppercase; all other characters are converted to lowercase.

```
"hello, world!".capitalize() == "Hello, world!"
"Hello, World!".capitalize() == "Hello, world!"
"".capitalize() == ""
```

***

## str.codepoints

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">codepoints</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Iterable</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

[string.codepoints](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·codepoints): returns an iterable of the unicode codepoint of a string.

`S.codepoints()` returns an iterable value containing the
sequence of integer Unicode code points encoded by the string S.
Each invalid code within the string is treated as if it encodes the
Unicode replacement character, U+FFFD.

By returning an iterable, not a list, the cost of decoding the string
is deferred until actually needed; apply `list(...)` to the result to
materialize the entire sequence.

```
list("Hello, 世界".codepoints()) == [72, 101, 108, 108, 111, 44, 32, 19990, 30028]
```

***

## str.count

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">count</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">needle</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">start</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">end</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.count](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·count): count the number of occurrences of a string in another string.

`S.count(sub[, start[, end]])` returns the number of occurrences of
`sub` within the string S, or, if the optional substring indices
`start` and `end` are provided, within the designated substring of S.
They are interpreted according to Skylark's [indexing conventions](https://github.com/bazelbuild/starlark/blob/master/spec.md#indexing).

This implementation does not count occurrence of `sub` in the string `S`
that overlap other occurrence of S (which can happen if some suffix of S
is a prefix of S). For instance, `"abababa".count("aba")` returns 2
for `[aba]a[aba]`, not counting the middle occurrence: `ab[aba]ba`
(this is following Python behavior).

```
"hello, world!".count("o") == 2
"abababa".count("aba") == 2
"hello, world!".count("o", 7, 12) == 1  # in "world"
```

***

## str.elems

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">elems</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Iterable</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

[string.elems](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·elems): returns an iterable of the bytes values of a string.

`S.elems()` returns an iterable value containing the
sequence of numeric bytes values in the string S.

To materialize the entire sequence of bytes, apply `list(...)` to the
result.

```
list("Hello, 世界".elems()) == ["H", "e", "l", "l", "o", ",", " ", "世", "界"]
```

***

## str.endswith

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">endswith</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">suffix</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/tuple">tuple</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span>, <span class="constant language python">...</span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.endswith](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·endswith): determine if a string ends with a given suffix.

`S.endswith(suffix)` reports whether the string S has the specified
suffix.

```
"filename.sky".endswith(".sky") == True
```

***

## str.find

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">find</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">needle</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">start</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">end</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.find](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·find): find a substring in a string.

`S.find(sub[, start[, end]])` returns the index of the first
occurrence of the substring `sub` within S.

If either or both of `start` or `end` are specified,
they specify a subrange of S to which the search should be restricted.
They are interpreted according to Skylark's [indexing
conventions](#indexing).

If no occurrence is found, `found` returns -1.

```
"bonbon".find("on") == 1
"bonbon".find("on", 2) == 4
"bonbon".find("on", 2, 5) == -1
```

***

## str.format

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">format</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    *<span class="variable parameter python">args</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Any</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    **<span class="variable parameter python">kwargs</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Any</a><span class="punctuation definition string end python">&#39;</span></span></span>
</span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.format](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·format): format a string.

`S.format(*args, **kwargs)` returns a version of the format string S
in which bracketed portions `{...}` are replaced
by arguments from `args` and `kwargs`.

Within the format string, a pair of braces `{{` or `}}` is treated as
a literal open or close brace.
Each unpaired open brace must be matched by a close brace `}`.
The optional text between corresponding open and close braces
specifies which argument to use and how to format it, and consists of
three components, all optional:
a field name, a conversion preceded by '`!`', and a format specifier
preceded by '`:`'.

```text
{field}
{field:spec}
{field!conv}
{field!conv:spec}
```

The *field name* may be either a decimal number or a keyword.
A number is interpreted as the index of a positional argument;
a keyword specifies the value of a keyword argument.
If all the numeric field names form the sequence 0, 1, 2, and so on,
they may be omitted and those values will be implied; however,
the explicit and implicit forms may not be mixed.

The *conversion* specifies how to convert an argument value `x` to a
string. It may be either `!r`, which converts the value using
`repr(x)`, or `!s`, which converts the value using `str(x)` and is
the default.

The *format specifier*, after a colon, specifies field width,
alignment, padding, and numeric precision.
Currently it must be empty, but it is reserved for future use.

```rust
"a {} c".format(3) == "a 3 c"
"a{x}b{y}c{}".format(1, x=2, y=3) == "a2b3c1"
"a{}b{}c".format(1, 2) == "a1b2c"
"({1}, {0})".format("zero", "one") == "(one, zero)"
"Is {0!r} {0!s}?".format("heterological") == "Is \"heterological\" heterological?"
```

***

## str.index

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">index</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">needle</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">start</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">end</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.index](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·index): search a substring inside a string, failing on not found.

`S.index(sub[, start[, end]])` returns the index of the first
occurrence of the substring `sub` within S, like `S.find`, except
that if the substring is not found, the operation fails.

```
"bonbon".index("on") == 1
"bonbon".index("on", 2) == 4
"bonbon".index("on", 2, 5)    # error: not found
```

***

## str.isalnum

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">isalnum</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.isalnum](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·isalnum): test if a string is composed only of letters and digits.

`S.isalnum()` reports whether the string S is non-empty and consists
only Unicode letters and digits.

```
"base64".isalnum() == True
"Catch-22".isalnum() == False
```

***

## str.isalpha

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">isalpha</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.isalpha](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·isalpha): test if a string is composed only of letters.

`S.isalpha()` reports whether the string S is non-empty and consists
only of Unicode letters.

```
"ABC".isalpha() == True
"Catch-22".isalpha() == False
"".isalpha() == False
```

***

## str.isdigit

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">isdigit</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.isdigit](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·isdigit): test if a string is composed only of digits.

`S.isdigit()` reports whether the string S is non-empty and consists
only of Unicode digits.

```
"123".isdigit() == True
"Catch-22".isdigit() == False
"".isdigit() == False
```

***

## str.islower

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">islower</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.islower](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·islower): test if all letters of a string are lowercase.

`S.islower()` reports whether the string S contains at least one cased
Unicode letter, and all such letters are lowercase.

```
"hello, world".islower() == True
"Catch-22".islower() == False
"123".islower() == False
```

***

## str.isspace

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">isspace</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.isspace](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·isspace): test if all characters of a string are whitespaces.

`S.isspace()` reports whether the string S is non-empty and consists
only of Unicode spaces.

```
"    ".isspace() == True
"\r\t\n".isspace() == True
"".isspace() == False
```

***

## str.istitle

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">istitle</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.istitle](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·istitle): test if the string is title cased.

`S.istitle()` reports whether the string S contains at least one cased
Unicode letter, and all such letters that begin a word are in title
case.

```
"Hello, World!".istitle() == True
"Catch-22".istitle() == True
"HAL-9000".istitle() == False
"123".istitle() == False
```

***

## str.isupper

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">isupper</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.isupper](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·isupper): test if all letters of a string are uppercase.

`S.isupper()` reports whether the string S contains at least one cased
Unicode letter, and all such letters are uppercase.

```
"HAL-9000".isupper() == True
"Catch-22".isupper() == False
"123".isupper() == False
```

***

## str.join

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">join</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">to_join</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Iterable</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.join](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·join): join elements with a separator.

`S.join(iterable)` returns the string formed by concatenating each
element of its argument, with a copy of the string S between
successive elements. The argument must be an iterable whose elements
are strings.

```
", ".join([]) == ""
", ".join(("x", )) == "x"
", ".join(["one", "two", "three"]) == "one, two, three"
"a".join("ctmrn".elems()) == "catamaran"
```

***

## str.lower

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">lower</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.lower](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·lower): convert a string to all lowercase.

`S.lower()` returns a copy of the string S with letters converted to
lowercase.

```
"Hello, World!".lower() == "hello, world!"
```

***

## str.lstrip

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">lstrip</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">chars</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.lstrip](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·lstrip): trim leading whitespaces.

`S.lstrip()` returns a copy of the string S with leading whitespace removed.
In most cases instead of passing an argument you should use `removeprefix`.

```
"  hello  ".lstrip() == "hello  "
"x!hello  ".lstrip("!x ") == "hello  "
```

***

## str.partition

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">partition</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">needle</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta group python"><span class="punctuation section group begin python">(</span><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="punctuation separator tuple python">,</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="punctuation separator tuple python">,</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="punctuation section group end python">)</span></span></span></span></code></pre>

[string.partition](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·partition): partition a string in 3 components

`S.partition(x = " ")` splits string S into three parts and returns them
as a tuple: the portion before the first occurrence of string `x`,
`x` itself, and the portion following it.
If S does not contain `x`, `partition` returns `(S, "", "")`.

`partition` fails if `x` is not a string, or is the empty string.

```
"one/two/three".partition("/") == ("one", "/", "two/three")
"one".partition("/") == ("one", "", "")
```

***

## str.removeprefix

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">removeprefix</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">prefix</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.removeprefix](https://docs.python.org/3.9/library/stdtypes.html#str.removeprefix): remove a prefix from a string. *Not part of standard Starlark.*

If the string starts with the prefix string, return `string[len(prefix):]`.
Otherwise, return a copy of the original string:

```
"Hello, World!".removeprefix("Hello") == ", World!"
"Hello, World!".removeprefix("Goodbye") == "Hello, World!"
"Hello".removeprefix("Hello") == ""
```

***

## str.removesuffix

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">removesuffix</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">suffix</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.removesuffix](https://docs.python.org/3.9/library/stdtypes.html#str.removesuffix): remove a prefix from a string. *Not part of standard Starlark.*

If the string starts with the prefix string, return `string[len(prefix):]`.
Otherwise, return a copy of the original string:

```
"Hello, World!".removesuffix("World!") == "Hello, "
"Hello, World!".removesuffix("World") == "Hello, World!"
"Hello".removesuffix("Hello") == ""
```

***

## str.replace

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">replace</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">old</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">new</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">count</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.replace](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·replace): replace all occurrences of a substring.

`S.replace(old, new[, count])` returns a copy of string S with all
occurrences of substring `old` replaced by `new`. If the optional
argument `count`, which must be an `int`, is non-negative, it
specifies a maximum number of occurrences to replace.

```
"banana".replace("a", "o") == "bonono"
"banana".replace("a", "o", 2) == "bonona"
"banana".replace("z", "x") == "banana"
"banana".replace("", "x") == "xbxaxnxaxnxax"
"banana".replace("", "x", 2) == "xbxanana"
"".replace("", "x") == "x"
"banana".replace("a", "o", -2)  # error: argument was negative
```

***

## str.rfind

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">rfind</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">needle</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">start</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">end</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.rfind](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·rfind): find the last index of a substring.

`S.rfind(sub[, start[, end]])` returns the index of the substring `sub`
within S, like `S.find`, except that `rfind` returns the index of
the substring's *last* occurrence.

```
"bonbon".rfind("on") == 4
"bonbon".rfind("on", None, 5) == 1
"bonbon".rfind("on", 2, 5) == -1
```

***

## str.rindex

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">rindex</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">needle</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">start</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">end</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.rindex](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·rindex): find the last index of a substring, failing on not found.

`S.rindex(sub[, start[, end]])` returns the index of the substring `sub`
within S, like `S.index`, except that `rindex` returns the index of
the substring's *last* occurrence.

```
"bonbon".rindex("on") == 4
"bonbon".rindex("on", None, 5) == 1  # in "bonbo"
"bonbon".rindex("on", 2, 5) #   error: not found
```

***

## str.rpartition

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">rpartition</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">needle</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta group python"><span class="punctuation section group begin python">(</span><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="punctuation separator tuple python">,</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="punctuation separator tuple python">,</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="punctuation section group end python">)</span></span></span></span></code></pre>

[string.rpartition](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·rpartition): partition a string in 3 elements.

`S.rpartition([x = ' '])` is like `partition`, but splits `S` at the
last occurrence of `x`.

```
"one/two/three".rpartition("/") == ("one/two", "/", "three")
"one".rpartition("/") == ("", "", "one")
```

***

## str.rsplit

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">rsplit</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">sep</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">maxsplit</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

[string.rsplit](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·rsplit): splits a string into substrings.

`S.rsplit([sep[, maxsplit]])` splits a string into substrings like
`S.split`, except that when a maximum number of splits is specified,
`rsplit` chooses the rightmost splits.

```
"banana".rsplit("n") == ["ba", "a", "a"]
"banana".rsplit("n", 1) == ["bana", "a"]
"one two  three".rsplit(None, 1) == ["one two", "three"]
```

***

## str.rstrip

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">rstrip</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">chars</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.rstrip](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·rstrip): trim trailing whitespace.

`S.rstrip()` returns a copy of the string S with trailing whitespace removed.
In most cases instead of passing an argument you should use `removesuffix`.

```
"  hello  ".rstrip() == "  hello"
"  hello!x".rstrip(" x!") == "  hello"
```

***

## str.split

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">split</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">sep</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">maxsplit</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

[string.split](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·split): split a string in substrings.

`S.split([sep [, maxsplit]])` returns the list of substrings of S,
splitting at occurrences of the delimiter string `sep`.

Consecutive occurrences of `sep` are considered to delimit empty
strings, so `'food'.split('o')` returns `['f', '', 'd']`.
Splitting an empty string with a specified separator returns `['']`.
If `sep` is the empty string, `split` fails.

If `sep` is not specified or is `None`, `split` uses a different
algorithm: it removes all leading spaces from S
(or trailing spaces in the case of `rsplit`),
then splits the string around each consecutive non-empty sequence of
Unicode white space characters.

If S consists only of white space, `split` returns the empty list.

If `maxsplit` is given and non-negative, it specifies a maximum number
of splits.

```
"one two  three".split() == ["one", "two", "three"]
"one two  three".split(" ") == ["one", "two", "", "three"]
"one two  three".split(None, 1) == ["one", "two  three"]
"banana".split("n") == ["ba", "a", "a"]
"banana".split("n", 1) == ["ba", "ana"]
```

***

## str.splitlines

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">splitlines</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">keepends</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">False</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

[string.splitlines](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·splitlines): return the list of lines of a string.

`S.splitlines([keepends])` returns a list whose elements are the
successive lines of S, that is, the strings formed by splitting S at
line terminators ('\n', '\r' or '\r\n').

The optional argument, `keepends`, is interpreted as a Boolean.
If true, line terminators are preserved in the result, though
the final element does not necessarily end with a line terminator.

```
"one\n\ntwo".splitlines() == ["one", "", "two"]
"one\n\ntwo".splitlines(True) == ["one\n", "\n", "two"]
"a\nb".splitlines() == ["a", "b"]
```

***

## str.startswith

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">startswith</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">prefix</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/tuple">tuple</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span>, <span class="constant language python">...</span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.startswith](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·startswith): test whether a string starts with a given prefix.

`S.startswith(suffix)` reports whether the string S has the specified
prefix.

```
"filename.sky".startswith("filename") == True
"filename.sky".startswith("sky") == False
'abc'.startswith(('a', 'A')) == True
'ABC'.startswith(('a', 'A')) == True
'def'.startswith(('a', 'A')) == False
```

***

## str.strip

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">strip</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">chars</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.strip](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·strip): trim leading and trailing whitespaces.

`S.strip()` returns a copy of the string S with leading and trailing
whitespace removed.

```
"  hello  ".strip() == "hello"
"xxhello!!".strip("x!") == "hello"
```

***

## str.title

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">title</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.title](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·title): convert a string to title case.

`S.title()` returns a copy of the string S with letters converted to
titlecase.

Letters are converted to uppercase at the start of words, lowercase
elsewhere.

```
"hElLo, WoRlD!".title() == "Hello, World!"
```

***

## str.upper

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">str</span></span>.<span class="entity name function python"><span class="meta generic-name python">upper</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[string.upper](https://github.com/bazelbuild/starlark/blob/master/spec.md#string·upper): convert a string to all uppercase.

`S.upper()` returns a copy of the string S with letters converted to
uppercase.

```
"Hello, World!".upper() == "HELLO, WORLD!"
```
