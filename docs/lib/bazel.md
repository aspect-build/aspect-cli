

## Bazel.build

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Bazel</span></span>.<span class="entity name function python"><span class="meta generic-name python">build</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    *<span class="variable parameter python">targets</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">events</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">list</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build_events/grpc">bazel.build.BuildEventSink</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">execution_logs</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">True</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">bazel_flags</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">list</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="meta structure list python"><span class="punctuation section list begin python">[</span><span class="punctuation section list end python">]</span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">bazel_verb</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="constant language python">None</span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">None</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build">bazel.build.Build</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Build targets within AXL with ctx.bazel.build(). The result is a `build`, which has `artifacts()` (TODO), `failures()` (TODO), and a `events()` functions that provide iterators to the artifacts, failures, events respectively.

Running `ctx.bazel.build()` does not block the starlark thread. Explicitly
call `.wait()` on the `build` type to wait until the build finishes.

You can pass in a single target or target pattern to build.

# Examples

```python
def impl(ctx):
    io = ctx.std.io
    build = ctx.bazel.build(
        "//target/to:build"
        events = True,
    )
    for event in build.events():
        if event.type == "progress":
            io.stdout.write(event.payload.stdout)
            io.stderr.write(event.payload.stderr)
```

***

## Bazel.query

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Bazel</span></span>.<span class="entity name function python"><span class="meta generic-name python">query</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/query">bazel.query.Query</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

The query system provides a programmatic interface for analyzing build dependencies and target relationships. Queries are constructed using a chain API and are lazily evaluated only when `.eval()` is explicitly called.

The entry point is `ctx.bazel.query()`, which returns a `query` for creating initial
query expressions. Most operations operate on `query` objects, which represent
sets of targets that can be filtered, transformed, and combined.

# Example

```starlark
# Query dependencies of a target
deps = ctx.bazel.query().targets("//myapp:main").deps()
all_deps = deps.eval()

# Chain multiple operations
sources = ctx.bazel.query().targets("//myapp:main")
    .deps()
    .kind("source file")
    .eval()
```
