

`function` **Command.arg**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Command</span></span>.<span class="entity name function python"><span class="meta generic-name python">arg</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">arg</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/str">str</a></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/typing">typing</a><span class="punctuation accessor dot python">.</span><a href="/lib/typing">Any</a></span></span></code></pre>

`function` **Command.args**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Command</span></span>.<span class="entity name function python"><span class="meta generic-name python">args</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">args</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/typing">typing</a><span class="punctuation accessor dot python">.</span><a href="/lib/typing">Any</a></span></span></code></pre>

`function` **Command.current\_dir**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Command</span></span>.<span class="entity name function python"><span class="meta generic-name python">current_dir</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">dir</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/str">str</a></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/typing">typing</a><span class="punctuation accessor dot python">.</span><a href="/lib/typing">Any</a></span></span></code></pre>

`function` **Command.env**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Command</span></span>.<span class="entity name function python"><span class="meta generic-name python">env</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">key</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/str">str</a></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">value</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/typing">typing</a><span class="punctuation accessor dot python">.</span><a href="/lib/typing">Any</a></span></span></code></pre>

`function` **Command.spawn**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Command</span></span>.<span class="entity name function python"><span class="meta generic-name python">spawn</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/std">std</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/process">process</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/process/child">Child</a></span></span></code></pre>

`function` **Command.stderr**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Command</span></span>.<span class="entity name function python"><span class="meta generic-name python">stderr</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">io</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/str">str</a></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/typing">typing</a><span class="punctuation accessor dot python">.</span><a href="/lib/typing">Any</a></span></span></code></pre>

Configuration for the child process's standard error (stderr) handle.

Defaults to \[`inherit`] when used with \[`spawn`] or \[`status`], and
defaults to \[`piped`] when used with \[`output`].

`function` **Command.stdin**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Command</span></span>.<span class="entity name function python"><span class="meta generic-name python">stdin</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">io</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/str">str</a></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/typing">typing</a><span class="punctuation accessor dot python">.</span><a href="/lib/typing">Any</a></span></span></code></pre>

Configuration for the child process's standard input (stdin) handle.

Defaults to \[`inherit`] when used with \[`spawn`] or \[`status`], and
defaults to \[`piped`] when used with \[`output`].

`function` **Command.stdout**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Command</span></span>.<span class="entity name function python"><span class="meta generic-name python">stdout</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">io</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/str">str</a></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/typing">typing</a><span class="punctuation accessor dot python">.</span><a href="/lib/typing">Any</a></span></span></code></pre>

Configuration for the child process's standard output (stdout) handle.

Defaults to \[`inherit`] when used with \[`spawn`] or \[`status`], and
defaults to \[`piped`] when used with \[`output`].
