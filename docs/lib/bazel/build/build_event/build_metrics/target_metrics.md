

## TargetMetrics.targets\_configured

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">targets_configured</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Number of targets/aspects configured during this build. Does not include targets/aspects that were configured on prior builds on this server and were cached. See BuildGraphMetrics below if you need that.

***

## TargetMetrics.targets\_configured\_not\_including\_aspects

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">targets_configured_not_including_aspects</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Number of configured targets analyzed during this build. Does not include aspects. Used mainly to allow consumers of targets\_configured, which used to not include aspects, to normalize across the Blaze release that switched targets\_configured to include aspects.

***

## TargetMetrics.targets\_loaded

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">targets_loaded</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

DEPRECATED No longer populated. It never measured what it was supposed to (targets loaded): it counted targets that were analyzed even if the underlying package had not changed. TODO(janakr): rename and remove.
