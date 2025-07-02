

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">dict</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="keyword operator unpacking sequence python">*</span><span class="variable parameter python">args</span><span class="punctuation separator parameters python">,</span> <span class="keyword operator unpacking mapping python">**</span><span class="variable parameter python">kwargs</span><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta qualified-name python"><span class="support type python">dict</span></span></span></span></code></pre>

[dict](https://github.com/bazelbuild/starlark/blob/master/spec.md#dict): creates a dictionary.

`dict` creates a dictionary. It accepts up to one positional argument,
which is interpreted as an iterable of two-element sequences
(pairs), each specifying a key/value pair in the
resulting dictionary.

`dict` also accepts any number of keyword arguments, each of which
specifies a key/value pair in the resulting dictionary; each keyword
is treated as a string.

```
dict() == {}
dict(**{'a': 1}) == {'a': 1}
dict({'a': 1}) == {'a': 1}
dict([(1, 2), (3, 4)]) == {1: 2, 3: 4}
dict([(1, 2), ['a', 'b']]) == {1: 2, 'a': 'b'}
dict(one=1, two=2) == {'one': 1, 'two': 2}
dict([(1, 2)], x=3) == {1: 2, 'x': 3}
dict([('x', 2)], x=3) == {'x': 3}
x = {'a': 1}
y = dict([('x', 2)], **x)
x == {'a': 1} and y == {'x': 2, 'a': 1}
```

***

## dict.clear

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">dict</span></span>.<span class="entity name function python"><span class="meta generic-name python">clear</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

[dict.clear](https://github.com/bazelbuild/starlark/blob/master/spec.md#dict·clear): clear a dictionary

`D.clear()` removes all the entries of dictionary D and returns `None`.
It fails if the dictionary is frozen or if there are active iterators.

```
x = {"one": 1, "two": 2}
x.clear()
x == {}
```

***

## dict.get

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">dict</span></span>.<span class="entity name function python"><span class="meta generic-name python">get</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">key</span><span class="punctuation separator parameters python">,</span> <span class="variable parameter python">default</span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"></span></span></code></pre>

[dict.get](https://github.com/bazelbuild/starlark/blob/master/spec.md#dict·get): return an element from the dictionary.

`D.get(key[, default])` returns the dictionary value corresponding to
the given key. If the dictionary contains no such value, `get`
returns `None`, or the value of the optional `default` parameter if
present.

`get` fails if `key` is unhashable.

```
x = {"one": 1, "two": 2}
x.get("one") == 1
x.get("three") == None
x.get("three", 0) == 0
```

***

## dict.items

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">dict</span></span>.<span class="entity name function python"><span class="meta generic-name python">items</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">list</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta group python"><span class="punctuation section group begin python">(</span><span class="meta qualified-name python"><span class="meta generic-name python">typing</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Any</span></span><span class="punctuation separator tuple python">,</span> <span class="meta qualified-name python"><span class="meta generic-name python">typing</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Any</span></span><span class="punctuation section group end python">)</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

[dict.items](https://github.com/bazelbuild/starlark/blob/master/spec.md#dict·items): get list of (key, value) pairs.

`D.items()` returns a new list of key/value pairs, one per element in
dictionary D, in the same order as they would be returned by a `for`
loop.

```
x = {"one": 1, "two": 2}
x.items() == [("one", 1), ("two", 2)]
```

***

## dict.keys

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">dict</span></span>.<span class="entity name function python"><span class="meta generic-name python">keys</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta qualified-name python"><span class="support type python">list</span></span></span></span></code></pre>

[dict.keys](https://github.com/bazelbuild/starlark/blob/master/spec.md#dict·keys): get the list of keys of the dictionary.

`D.keys()` returns a new list containing the keys of dictionary D, in
the same order as they would be returned by a `for` loop.

```
x = {"one": 1, "two": 2}
x.keys() == ["one", "two"]
```

***

## dict.pop

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">dict</span></span>.<span class="entity name function python"><span class="meta generic-name python">pop</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">key</span><span class="punctuation separator parameters python">,</span> <span class="variable parameter python">default</span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"></span></span></code></pre>

[dict.pop](https://github.com/bazelbuild/starlark/blob/master/spec.md#dict·pop): return an element and remove it from a dictionary.

`D.pop(key[, default])` returns the value corresponding to the specified
key, and removes it from the dictionary.  If the dictionary contains no
such value, and the optional `default` parameter is present, `pop`
returns that value; otherwise, it fails.

`pop` fails if `key` is unhashable, or the dictionary is frozen or has
active iterators.

```
x = {"one": 1, "two": 2}
x.pop("one") == 1
x == {"two": 2}
x.pop("three", 0) == 0
x.pop("three", None) == None
```

Failure:

```
{'one': 1}.pop('four')   # error: not found
```

***

## dict.popitem

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">dict</span></span>.<span class="entity name function python"><span class="meta generic-name python">popitem</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta group python"><span class="punctuation section group begin python">(</span><span class="meta qualified-name python"><span class="meta generic-name python">typing</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Any</span></span><span class="punctuation separator tuple python">,</span> <span class="meta qualified-name python"><span class="meta generic-name python">typing</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Any</span></span><span class="punctuation section group end python">)</span></span></span></span></code></pre>

[dict.popitem](https://github.com/bazelbuild/starlark/blob/master/spec.md#dict·popitem): returns and removes the first key/value pair of a dictionary.

`D.popitem()` returns the first key/value pair, removing it from the
dictionary.

`popitem` fails if the dictionary is empty, frozen, or has active
iterators.

```
x = {"one": 1, "two": 2}
x.popitem() == ("one", 1)
x.popitem() == ("two", 2)
x == {}
```

Failure:

```
{}.popitem()   # error: empty dict
```

***

## dict.setdefault

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">dict</span></span>.<span class="entity name function python"><span class="meta generic-name python">setdefault</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">key</span><span class="punctuation separator parameters python">,</span> <span class="variable parameter python">default</span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"></span></span></code></pre>

[dict.setdefault](https://github.com/bazelbuild/starlark/blob/master/spec.md#dict·setdefault): get a value from a dictionary, setting it to a new value if not present.

`D.setdefault(key[, default])` returns the dictionary value
corresponding to the given key. If the dictionary contains no such
value, `setdefault`, like `get`, returns `None` or the value of the
optional `default` parameter if present; `setdefault` additionally
inserts the new key/value entry into the dictionary.

`setdefault` fails if the key is unhashable or if the dictionary is
frozen.

```
x = {"one": 1, "two": 2}
x.setdefault("one") == 1
x.setdefault("three", 0) == 0
x == {"one": 1, "two": 2, "three": 0}
x.setdefault("four") == None
x == {"one": 1, "two": 2, "three": 0, "four": None}
```

***

## dict.update

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">dict</span></span>.<span class="entity name function python"><span class="meta generic-name python">update</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">pairs</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="meta generic-name python">typing</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Iterable</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta group python"><span class="punctuation section group begin python">(</span><span class="meta qualified-name python"><span class="meta generic-name python">typing</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Any</span></span><span class="punctuation separator tuple python">,</span> <span class="meta qualified-name python"><span class="meta generic-name python">typing</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Any</span></span><span class="punctuation section group end python">)</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="support type python">dict</span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
    **<span class="variable parameter python">kwargs</span><span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

[dict.update](https://github.com/bazelbuild/starlark/blob/master/spec.md#dict·update): update values in the dictionary.

`D.update([pairs][, name=value[, ...])` makes a sequence of key/value
insertions into dictionary D, then returns `None.`

If the positional argument `pairs` is present, it must be `None`,
another `dict`, or some other iterable.
If it is another `dict`, then its key/value pairs are inserted into D.
If it is an iterable, it must provide a sequence of pairs (or other
iterables of length 2), each of which is treated as a key/value pair
to be inserted into D.

For each `name=value` argument present, the name is converted to a
string and used as the key for an insertion into D, with its
corresponding value being `value`.

`update` fails if the dictionary is frozen.

```
x = {}
x.update([("a", 1), ("b", 2)], c=3)
x.update({"d": 4})
x.update(e=5)
x == {"a": 1, "b": 2, "c": 3, "d": 4, "e": 5}
```

***

## dict.values

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">dict</span></span>.<span class="entity name function python"><span class="meta generic-name python">values</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta qualified-name python"><span class="support type python">list</span></span></span></span></code></pre>

[dict.values](https://github.com/bazelbuild/starlark/blob/master/spec.md#dict·values): get the list of values of the dictionary.

`D.values()` returns a new list containing the dictionary's values, in
the same order as they would be returned by a `for` loop over the
dictionary.

```
x = {"one": 1, "two": 2}
x.values() == [1, 2]
```
