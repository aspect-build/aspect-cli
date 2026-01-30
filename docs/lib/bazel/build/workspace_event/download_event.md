

`property` **DownloadEvent.executable**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">executable</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

whether to make the resulting file executable

`property` **DownloadEvent.integrity**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">integrity</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

checksum in Subresource Integrity format, if specified

`property` **DownloadEvent.output**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">output</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

Output file

`property` **DownloadEvent.sha256**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">sha256</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

sha256, if specified

`property` **DownloadEvent.url**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">url</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Url to download from. If multiple, treated as mirrors
