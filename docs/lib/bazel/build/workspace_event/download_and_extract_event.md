

`property` **DownloadAndExtractEvent.integrity**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadAndExtractEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">integrity</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

checksum in Subresource Integrity format, if specified

`property` **DownloadAndExtractEvent.output**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadAndExtractEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">output</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

Output file

`property` **DownloadAndExtractEvent.rename\_files**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadAndExtractEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">rename_files</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/dict">dict</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a>, <a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Files to rename during extraction.

`property` **DownloadAndExtractEvent.sha256**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadAndExtractEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">sha256</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

sha256, if specified

`property` **DownloadAndExtractEvent.strip\_prefix**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadAndExtractEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">strip_prefix</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

A directory prefix to strip from extracted files.

`property` **DownloadAndExtractEvent.type**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadAndExtractEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">type</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

Archive type, if specified. Otherwise, inferred from URL.

`property` **DownloadAndExtractEvent.url**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">DownloadAndExtractEvent</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">url</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Url(s) to download from
