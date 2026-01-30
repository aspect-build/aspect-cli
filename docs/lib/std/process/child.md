

`function` **Child.kill**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Child</span></span>.<span class="entity name function python"><span class="meta generic-name python">kill</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib">None</a></span></span></code></pre>

Forces the child process to exit. If the child has already exited, its a no-op.

This is equivalent to sending a SIGKILL on Unix platforms.

`function` **Child.stderr**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Child</span></span>.<span class="entity name function python"><span class="meta generic-name python">stderr</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/std">std</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/io">io</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/io/readable">Readable</a></span></span></code></pre>

The handle for reading from the child’s standard error (stderr), if it has been captured. Calling this function more than once will yield error.

`function` **Child.stdin**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Child</span></span>.<span class="entity name function python"><span class="meta generic-name python">stdin</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/std">std</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/io">io</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/io/writable">Writable</a></span></span></code></pre>

The handle for writing to the child’s standard input (stdin), if it has been captured. Calling this function more than once will yield error.

`function` **Child.stdout**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Child</span></span>.<span class="entity name function python"><span class="meta generic-name python">stdout</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/std">std</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/io">io</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/io/readable">Readable</a></span></span></code></pre>

The handle for reading from the child’s standard output (stdout), if it has been captured. Calling this function more than once will yield error.

`function` **Child.wait**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Child</span></span>.<span class="entity name function python"><span class="meta generic-name python">wait</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/std">std</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/process">process</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/process/exit_status">ExitStatus</a></span></span></code></pre>

Waits for the child to exit completely, returning the status that it exited with. This function will continue to have the same return value after it has been called at least once.

The stdin handle to the child process, if any, will be closed
before waiting. This helps avoid deadlock: it ensures that the
child does not block waiting for input from the parent, while
the parent waits for the child to exit.

`function` **Child.wait\_with\_output**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Child</span></span>.<span class="entity name function python"><span class="meta generic-name python">wait_with_output</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/std">std</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/process">process</a><span class="punctuation accessor dot python">.</span><a href="/lib/std/process/output">Output</a></span></span></code></pre>

WARNING: Calling `wait_with_output` consumes the child instance, causing errors on subsequent calls to other methods.

Simultaneously waits for the child to exit and collect all remaining
output on the stdout/stderr handles, returning an `Output`
instance.

The stdin handle to the child process, if any, will be closed
before waiting. This helps avoid deadlock: it ensures that the
child does not block waiting for input from the parent, while
the parent waits for the child to exit.

By default, stdin, stdout and stderr are inherited from the parent.
In order to capture the output into this `Result<Output>` it is
necessary to create new pipes between parent and child. Use
`stdout('piped')` or `stderr('piped')`, respectively.

`property` **Child.id**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Child</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">id</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

Returns the OS-assigned process identifier associated with this child.
