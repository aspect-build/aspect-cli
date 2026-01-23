

## MemoryMetrics.garbage\_metrics

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">MemoryMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">garbage_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/build_metrics/memory_metrics/garbage_metrics">garbage_metrics</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

***

## MemoryMetrics.peak\_post\_gc\_heap\_size

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">MemoryMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">peak_post_gc_heap_size</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Size of the peak JVM heap size in bytes post GC. Note that this reports 0 if there was no major GC during the build.

***

## MemoryMetrics.peak\_post\_gc\_tenured\_space\_heap\_size

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">MemoryMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">peak_post_gc_tenured_space_heap_size</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Size of the peak tenured space JVM heap size event in bytes post GC. Note that this reports 0 if there was no major GC during the build.

***

## MemoryMetrics.used\_heap\_size\_post\_build

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">MemoryMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">used_heap_size_post_build</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Size of the JVM heap post build in bytes. This is only collected if --memory\_profile is set, since it forces a full GC.
