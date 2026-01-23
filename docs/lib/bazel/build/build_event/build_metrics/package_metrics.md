

`property` **PackageMetrics.package\_load\_metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">PackageMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">package_load_metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">package_load_metrics</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Loading time metrics per package.

`property` **PackageMetrics.packages\_loaded**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">PackageMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">packages_loaded</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

Number of BUILD files (aka packages) successfully loaded during this build.

\[For Bazel binaries built at source states] Before Dec 2021, this value
was the number of packages attempted to be loaded, for a particular
definition of "attempted".

After Dec 2021, this value would sometimes overcount because the same
package could sometimes be attempted to be loaded multiple times due to
memory pressure.

After Feb 2022, this value is the number of packages successfully
loaded.
