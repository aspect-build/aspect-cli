

## set.add

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">set</span></span>.<span class="entity name function python"><span class="meta generic-name python">add</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">value</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Any</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Add an item to the set. `# starlark::assert::is_true(r#" x = set([1, 2, 3]) x.add(4) x == set([1, 2, 3, 4]) # "#);`

***

## set.clear

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">set</span></span>.<span class="entity name function python"><span class="meta generic-name python">clear</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

***

## set.difference

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">set</span></span>.<span class="entity name function python"><span class="meta generic-name python">difference</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">other</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Iterable</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/set">set</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Any</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

Returns a new set with elements unique the set when compared to the specified iterable. `# starlark::assert::is_true(r#" x = set([1, 2, 3]) y = [3, 4, 5] x.difference(y) == set([1, 2]) # "#);`

***

## set.discard

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">set</span></span>.<span class="entity name function python"><span class="meta generic-name python">discard</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">value</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Any</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Remove the item from the set. It does nothing if there is no such item.

`discard` fails if the key is unhashable or if the dictionary is
frozen.
Time complexity of this operation is *O(N)* where *N* is the number of entries in the set.

```
x = set([1, 2, 3])
x.discard(2)
x == set([1, 3])
```

A subsequent call to `x.discard(2)` would do nothing.

```
x = set([1, 2, 3])
x.discard(2)
x.discard(2)
x == set([1, 3])
```

***

## set.intersection

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">set</span></span>.<span class="entity name function python"><span class="meta generic-name python">intersection</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">other</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Iterable</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/set">set</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Any</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

Return a new set with elements common to the set and all others. Unlike Python does not support variable number of arguments. `# starlark::assert::is_true(r#" x = set([1, 2, 3]) y = [3, 4, 5] x.intersection(y) == set([3]) # "#);`

***

## set.issubset

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">set</span></span>.<span class="entity name function python"><span class="meta generic-name python">issubset</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">other</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Iterable</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Test whether every element in the set is in other iterable. `# starlark::assert::is_true(r#" x = set([1, 2, 3]) y = [3, 1, 2] x.issubset(y) # "#);`

***

## set.issuperset

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">set</span></span>.<span class="entity name function python"><span class="meta generic-name python">issuperset</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">other</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Iterable</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Test whether every element other iterable is in the set. `# starlark::assert::is_true(r#" x = set([1, 2, 3]) y = [1, 3] x.issuperset(y) == True # "#);`

***

## set.pop

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">set</span></span>.<span class="entity name function python"><span class="meta generic-name python">pop</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Any</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Removes and returns the **last** element of a set.

`S.pop()` removes and returns the last element of the set S.

`pop` fails if the set is empty, or if the set is frozen or has active iterators.
Time complexity of this operation is *O(1)*.

```
x = set([1, 2, 3])
x.pop() == 3
x.pop() == 2
x == set([1])
```

***

## set.remove

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">set</span></span>.<span class="entity name function python"><span class="meta generic-name python">remove</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">value</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Any</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Remove the item from the set. It raises an error if there is no such item.

`remove` fails if the key is unhashable or if the dictionary is
frozen.
Time complexity of this operation is *O(N)* where *N* is the number of entries in the set.

```
x = set([1, 2, 3])
x.remove(2)
x == set([1, 3])
```

A subsequent call to `x.remove(2)` would yield an error because the
element won't be found.

```
x = set([1, 2, 3])
x.remove(2)
x.remove(2) # error: not found
```

***

## set.symmetric\_difference

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">set</span></span>.<span class="entity name function python"><span class="meta generic-name python">symmetric_difference</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">other</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Iterable</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/set">set</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Any</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

Returns a new set with elements in either the set or the specified iterable but not both. `# starlark::assert::is_true(r#" x = set([1, 2, 3]) y = [3, 4, 5] x.symmetric_difference(y) == set([1, 2, 4, 5]) # "#);`

***

## set.union

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">set</span></span>.<span class="entity name function python"><span class="meta generic-name python">union</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">other</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Iterable</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/set">set</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Any</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

Return a new set with elements from the set and all others. Unlike Python does not support variable number of arguments. `# starlark::assert::is_true(r#" x = set([1, 2, 3]) y = [3, 4, 5] x.union(y) == set([1, 2, 3, 4, 5]) # "#);`

***

## set.update

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">set</span></span>.<span class="entity name function python"><span class="meta generic-name python">update</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">other</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/typing">typing.Iterable</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Update the set by adding items from an iterable. `# starlark::assert::is_true(r#" x = set([1, 3, 2]) x.update([4, 3]) list(x) == [1, 3, 2, 4] # "#);`
