

`function` **Template.handlebars**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Template</span></span>.<span class="entity name function python"><span class="meta generic-name python">handlebars</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">template</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/str">str</a></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
    *<span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">data</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/dict">dict</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a>, <a href="/lib/typing">typing</a><span class="punctuation accessor dot python">.</span><a href="/lib/typing">Any</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span>
</span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/str">str</a></span></span></code></pre>

Renders a Handlebars template with the provided data.

**Parameters**

* `template`: The Handlebars template string.
* `data`: A dictionary of data to render the template with.

**Returns**
The rendered template as a string.

**Example**

```starlark
result = ctx.template.handlebars("Hello, {{name}}!", {"name": "World"})
```

`function` **Template.jinja2**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Template</span></span>.<span class="entity name function python"><span class="meta generic-name python">jinja2</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">template</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/str">str</a></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
    *<span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">data</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/dict">dict</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a>, <a href="/lib/typing">typing</a><span class="punctuation accessor dot python">.</span><a href="/lib/typing">Any</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span>
</span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/str">str</a></span></span></code></pre>

Renders a Jinja2 template with the provided data.

**Parameters**

* `template`: The Jinja2 template string.
* `data`: A dictionary of data to render the template with.

**Returns**
The rendered template as a string.

**Example**

```starlark
result = ctx.template.jinja2("Hello, {{ name }}!", {"name": "World"})
```

`function` **Template.liquid**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Template</span></span>.<span class="entity name function python"><span class="meta generic-name python">liquid</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">template</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/str">str</a></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
    *<span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">data</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/dict">dict</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a>, <a href="/lib/typing">typing</a><span class="punctuation accessor dot python">.</span><a href="/lib/typing">Any</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span>
</span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/str">str</a></span></span></code></pre>

Renders a Liquid template with the provided data.

**Parameters**

* `template`: The Liquid template string.
* `data`: A dictionary of data to render the template with.

**Returns**
The rendered template as a string.

**Example**

```starlark
result = ctx.template.liquid("Hello, {{ name }}!", {"name": "World"})
```
