

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">list</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">a</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta qualified-name python"><span class="meta generic-name python">typing</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Iterable</span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta qualified-name python"><span class="support type python">list</span></span></span></span></code></pre>

[list](https://github.com/bazelbuild/starlark/blob/master/spec.md#list): construct a list.

`list(x)` returns a new list containing the elements of the
iterable sequence x.

With no argument, `list()` returns a new empty list.

```
list()        == []
list((1,2,3)) == [1, 2, 3]
list("strings are not iterable") # error: not supported
```

***

## list.append

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">list</span></span>.<span class="entity name function python"><span class="meta generic-name python">append</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">el</span><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

[list.append](https://github.com/bazelbuild/starlark/blob/master/spec.md#list·append): append an element to a list.

`L.append(x)` appends `x` to the list L, and returns `None`.

`append` fails if the list is frozen or has active iterators.

```
x = []
x.append(1)
x.append(2)
x.append(3)
x == [1, 2, 3]
```

***

## list.clear

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">list</span></span>.<span class="entity name function python"><span class="meta generic-name python">clear</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

[list.clear](https://github.com/bazelbuild/starlark/blob/master/spec.md#list·clear): clear a list

`L.clear()` removes all the elements of the list L and returns `None`.
It fails if the list is frozen or if there are active iterators.

```
x = [1, 2, 3]
x.clear()
x == []
```

***

## list.extend

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">list</span></span>.<span class="entity name function python"><span class="meta generic-name python">extend</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">other</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta qualified-name python"><span class="meta generic-name python">typing</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Iterable</span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

[list.extend](https://github.com/bazelbuild/starlark/blob/master/spec.md#list·extend): extend a list with another iterable's content.

`L.extend(x)` appends the elements of `x`, which must be iterable, to
the list L, and returns `None`.

`extend` fails if `x` is not iterable, or if the list L is frozen or has
active iterators.

```
x = []
x.extend([1, 2, 3])
x.extend(["foo"])
x == [1, 2, 3, "foo"]
```

***

## list.index

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">list</span></span>.<span class="entity name function python"><span class="meta generic-name python">index</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">needle</span><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">start</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="constant language python">None</span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">end</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="constant language python">None</span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

[list.index](https://github.com/bazelbuild/starlark/blob/master/spec.md#list·index): get the index of an element in the list.

`L.index(x[, start[, end]])` finds `x` within the list L and returns its
index.

The optional `start` and `end` parameters restrict the portion of
list L that is inspected.  If provided and not `None`, they must be list
indices of type `int`. If an index is negative, `len(L)` is effectively
added to it, then if the index is outside the range `[0:len(L)]`, the
nearest value within that range is used; see [Indexing](#indexing).

`index` fails if `x` is not found in L, or if `start` or `end`
is not a valid index (`int` or `None`).

```
x = ["b", "a", "n", "a", "n", "a"]
x.index("a") == 1      # bAnana
x.index("a", 2) == 3   # banAna
x.index("a", -2) == 5  # bananA
```

***

## list.insert

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">list</span></span>.<span class="entity name function python"><span class="meta generic-name python">insert</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">index</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> <span class="variable parameter python">el</span><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

[list.insert](https://github.com/bazelbuild/starlark/blob/master/spec.md#list·insert): insert an element in a list.

`L.insert(i, x)` inserts the value `x` in the list L at index `i`,
moving higher-numbered elements along by one.  It returns `None`.

As usual, the index `i` must be an `int`. If its value is negative,
the length of the list is added, then its value is clamped to the
nearest value in the range `[0:len(L)]` to yield the effective index.

`insert` fails if the list is frozen or has active iterators.

```
x = ["b", "c", "e"]
x.insert(0, "a")
x.insert(-1, "d")
x == ["a", "b", "c", "d", "e"]
```

***

## list.pop

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">list</span></span>.<span class="entity name function python"><span class="meta generic-name python">pop</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">index</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"></span></span></code></pre>

[list.pop](https://github.com/bazelbuild/starlark/blob/master/spec.md#list·pop): removes and returns the last element of a list.

`L.pop([index])` removes and returns the last element of the list L, or,
if the optional index is provided, at that index.

`pop` fails if the index is negative or not less than the length of
the list, of if the list is frozen or has active iterators.

```
x = [1, 2, 3]
x.pop() == 3
x.pop() == 2
x == [1]
```

***

## list.remove

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">list</span></span>.<span class="entity name function python"><span class="meta generic-name python">remove</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">needle</span><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

[list.remove](https://github.com/bazelbuild/starlark/blob/master/spec.md#list·remove): remove a value from a list

`L.remove(x)` removes the first occurrence of the value `x` from the
list L, and returns `None`.

`remove` fails if the list does not contain `x`, is frozen, or has
active iterators.

```
x = [1, 2, 3, 2]
x.remove(2)
x == [1, 3, 2]
x.remove(2)
x == [1, 3]
```

A subsequent call to `x.remove(2)` would yield an error because the
element won't be found.

```
x = [1, 2, 3, 2]
x.remove(2)
x.remove(2)
x.remove(2) # error: not found
```
