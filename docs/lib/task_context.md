

`function` **TaskContext.http**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">TaskContext</span></span>.<span class="entity name function python"><span class="meta generic-name python">http</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/http">Http</a></span></span></code></pre>

The `http` attribute provides a programmatic interface for making HTTP requests. It is used to fetch data from remote servers and can be used in conjunction with other aspects to perform complex data processing tasks.

**Example**

```starlark
**Fetch** data from a remote server
data = ctx.http().get("https://example.com/data.json").block()
```

`property` **TaskContext.args**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TaskContext</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">args</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/task_args">TaskArgs</a></span></code></pre>

Access to arguments provided by the caller.

`property` **TaskContext.bazel**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TaskContext</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">bazel</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bazel">bazel</a><span class="punctuation accessor dot python">.</span><a href="/lib/bazel/bazel">Bazel</a></span></code></pre>

Access to Bazel functionality.

`property` **TaskContext.config**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TaskContext</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">config</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/typing">typing</a><span class="punctuation accessor dot python">.</span><a href="/lib/typing">Any</a></span></code></pre>

Access to the task configuration.

`property` **TaskContext.std**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TaskContext</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">std</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/std">std</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/std">Std</a></span></code></pre>

Standard library is the foundation of powerful AXL tasks.

`property` **TaskContext.template**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TaskContext</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">template</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/template">Template</a></span></code></pre>

Expand template files.
