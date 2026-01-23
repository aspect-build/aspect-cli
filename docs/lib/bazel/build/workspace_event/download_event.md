

## DownloadEvent.executable

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">executable</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

whether to make the resulting file executable

***

## DownloadEvent.integrity

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">integrity</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

checksum in Subresource Integrity format, if specified

***

## DownloadEvent.output

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">output</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Output file

***

## DownloadEvent.sha256

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">sha256</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

sha256, if specified

***

## DownloadEvent.url

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">url</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Url to download from. If multiple, treated as mirrors
