

## ConfigContext.bazel

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ConfigContext</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">bazel</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel">bazel.Bazel</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Access to Bazel functionality.

***

## ConfigContext.http

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">ConfigContext</span></span>.<span class="entity name function python"><span class="meta generic-name python">http</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/http">Http</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

The `http` attribute provides a programmatic interface for making HTTP requests. It is used to fetch data from remote servers and can be used in conjunction with other aspects to perform complex data processing tasks.

# Example

```starlark
# Fetch data from a remote server
data = ctx.http().get("https://example.com/data.json").block()
```

***

## ConfigContext.std

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ConfigContext</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">std</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/std">std.Std</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Standard library is the foundation of powerful AXL tasks.

***

## ConfigContext.template

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ConfigContext</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">template</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/template">Template</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Expand template files.

***

## ConfigContext.wasm

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ConfigContext</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">wasm</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/wasm">wasm.Wasm</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

EXPERIMENTAL! Run wasm programs within tasks.
