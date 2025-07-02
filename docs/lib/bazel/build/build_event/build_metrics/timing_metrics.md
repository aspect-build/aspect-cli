

## TimingMetrics.actions\_execution\_start\_in\_ms

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TimingMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">actions_execution_start_in_ms</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The elapsed wall time in milliseconds until the first action execution started (excluding workspace status actions).

***

## TimingMetrics.analysis\_phase\_time\_in\_ms

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TimingMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">analysis_phase_time_in_ms</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The elapsed wall time in milliseconds during the analysis phase. When analysis and execution phases are interleaved, this measures the elapsed time from the first analysis work to the last.

***

## TimingMetrics.cpu\_time\_in\_ms

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TimingMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">cpu_time_in_ms</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

For Skymeld, it's possible that analysis\_phase\_time\_in\_ms + execution\_phase\_time\_in\_ms >= wall\_time\_in\_ms

The CPU time in milliseconds consumed during this build.

***

## TimingMetrics.execution\_phase\_time\_in\_ms

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TimingMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">execution_phase_time_in_ms</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The elapsed wall time in milliseconds during the execution phase. When analysis and execution phases are interleaved, this measures the elapsed time from the first action execution (excluding workspace status actions) to the last.

***

## TimingMetrics.wall\_time\_in\_ms

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TimingMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">wall_time_in_ms</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The elapsed wall time in milliseconds during this build.
