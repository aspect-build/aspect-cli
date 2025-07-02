

## ConfigurationId.id

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ConfigurationId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">id</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Identifier of the configuration; users of the protocol should not make any assumptions about it having any structure, or equality of the identifier between different streams.

A value of "none" means the null configuration. It is used for targets
that are not configurable, for example, source files.
