

`type` [ExitCode](/lib/bazel/build/build_event/build_finished/exit_code)

`type` [AnomalyReport](/lib/bazel/build/build_event/build_finished/anomaly_report)

`property` **BuildFinished.exit\_code**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildFinished</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">exit_code</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">exit_code</span></span></span></code></pre>

The overall status of the build. A build was successful iff ExitCode.code equals 0.

`property` **BuildFinished.failure\_detail**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildFinished</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">failure_detail</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">failure_detail</span></span></span></code></pre>

Only populated if success = false, and sometimes not even then.
