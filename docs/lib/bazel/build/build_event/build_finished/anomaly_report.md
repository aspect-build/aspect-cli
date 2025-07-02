

## AnomalyReport.was\_suspended

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AnomalyReport</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">was_suspended</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Was the build suspended at any time during the build. Examples of suspensions are SIGSTOP, or the hardware being put to sleep. If was\_suspended is true, then most of the timings for this build are suspect. NOTE: This is no longer set and is deprecated.
