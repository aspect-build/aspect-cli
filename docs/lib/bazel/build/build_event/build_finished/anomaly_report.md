

`property` **AnomalyReport.was\_suspended**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">AnomalyReport</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">was_suspended</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

Was the build suspended at any time during the build. Examples of suspensions are SIGSTOP, or the hardware being put to sleep. If was\_suspended is true, then most of the timings for this build are suspect. NOTE: This is no longer set and is deprecated.
