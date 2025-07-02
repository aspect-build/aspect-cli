

## BuildFinished.exit\_code

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildFinished</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">exit_code</span></span><span class="punctuation separator annotation variable python">:</span> <span class="constant language python">None</span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/build_finished/exit_code">exit_code</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The overall status of the build. A build was successful iff ExitCode.code equals 0.

***

## BuildFinished.failure\_detail

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildFinished</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">failure_detail</span></span><span class="punctuation separator annotation variable python">:</span> <span class="constant language python">None</span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">failure_detail</span></span></span></code></pre>

Only populated if success = false, and sometimes not even then.
