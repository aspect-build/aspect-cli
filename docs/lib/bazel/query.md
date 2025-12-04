

## Query.eval

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Query</span></span>.<span class="entity name function python"><span class="meta generic-name python">eval</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="meta generic-name python">typing</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Iterable</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/query/generated_file">generated_file</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/query/package_group">package_group</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/query/rule">rule</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/query/source_file">source_file</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

The query system provides a programmatic interface for analyzing build dependencies and target relationships. Queries are constructed using a chain API and are lazily evaluated only when `.eval()` is explicitly called.

The entry point is `ctx.bazel.query()`, which returns a `query` for creating initial
query expressions. Most operations operate on `query` objects, which represent
sets of targets that can be filtered, transformed, and combined.

**Example**

```starlark
**Query** dependencies of a target
deps = ctx.bazel.query().targets("//myapp:main").deps()
all_deps: target_set = deps.eval()

**Chain** multiple operations
sources = ctx.bazel.query().targets("//myapp:main")
    .deps()
    .kind("source file")
    .eval()
```

***

## Query.raw

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Query</span></span>.<span class="entity name function python"><span class="meta generic-name python">raw</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">expr</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/query">bazel.query.Query</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Replaces the query `expression` with a raw query expression string.

This escape hatch allows direct use of the underlying query language for complex cases,
while still supporting further chaining.

```starlark
**Complex** intersection query
complex = ctx.bazel.query().raw("deps(//foo) intersect kind('test', //bar:*)")

**Path**-based query
path_query = ctx.bazel.query().raw("somepath(//start, //end)")

**Chaining** after raw
filtered = complex.kind("source file")
```
