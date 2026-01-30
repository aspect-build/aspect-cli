

`property` **ExecuteEvent.arguments**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecuteEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">arguments</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Command line arguments, with the first one being the command to execute.

`property` **ExecuteEvent.environment**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecuteEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">environment</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/dict">dict</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a>, <a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Environment variables set for the execution. Note that this includes variables specified by the user (as an input to Execute command), as well as variables set indirectly through the rule environment

`property` **ExecuteEvent.output\_directory**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecuteEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">output_directory</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

Directory that would contain the output of the command.

`property` **ExecuteEvent.quiet**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecuteEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">quiet</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

True if quiet execution was requested.

`property` **ExecuteEvent.timeout\_seconds**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ExecuteEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">timeout_seconds</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

Timeout used for the command
