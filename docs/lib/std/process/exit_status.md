

`property` **ExitStatus.code**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExitStatus</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">code</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/int">int</a></span></code></pre>

Returns the exit code of the process, if any.

In Unix terms the return value is the **exit status**: the value passed to `exit`, if the
process finished by calling `exit`.  Note that on Unix the exit status is truncated to 8
bits, and that values that didn't come from a program's call to `exit` may be invented by the
runtime system (often, for example, 255, 254, 127 or 126).

On Unix, this will return `None` if the process was terminated by a signal.

`property` **ExitStatus.signal**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExitStatus</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">signal</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/int">int</a></span></code></pre>

If the process was terminated by a signal, returns that signal.

In other words, if `WIFSIGNALED`, this returns `WTERMSIG`.

Avability: UNIX

`property` **ExitStatus.stopped\_signal**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExitStatus</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">stopped_signal</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/int">int</a></span></code></pre>

If the process was stopped by a signal, returns that signal.

In other words, if `WIFSTOPPED`, this returns `WSTOPSIG`.  This is only possible if the status came from
a `wait` system call which was passed `WUNTRACED`, and was then converted into an `ExitStatus`.

Avability: UNIX

`property` **ExitStatus.success**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExitStatus</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">success</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

Was termination successful? Signal termination is not considered a success, and success is defined as a zero exit status.
