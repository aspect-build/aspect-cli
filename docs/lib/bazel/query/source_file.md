

## SourceFile.feature

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SourceFile</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">feature</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The package-level features enabled for this package. Only present if the SourceFile represents a BUILD file.

***

## SourceFile.license

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SourceFile</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">license</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/query/license">license</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

License attribute for the file.

***

## SourceFile.location

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SourceFile</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">location</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The location of the source file.  This is a path with a line number and a column number not a label in the build system.

***

## SourceFile.name

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SourceFile</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">name</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The name of the source file (a label).

***

## SourceFile.package\_contains\_errors

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SourceFile</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">package_contains_errors</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

True if the package contains an error. Only present if the SourceFile represents a BUILD file.

***

## SourceFile.package\_group

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SourceFile</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">package_group</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Labels of package groups that are mentioned in the visibility declaration for this source file.

***

## SourceFile.subinclude

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SourceFile</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">subinclude</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Labels of .bzl (Starlark) files that are transitively loaded in this BUILD file. This is present only when the SourceFile represents a BUILD file that loaded .bzl files. TODO(bazel-team): Rename this field.

***

## SourceFile.visibility\_label

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">SourceFile</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">visibility_label</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/list">list</a><span class="punctuation definition string end python">&#39;</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

Labels mentioned in the visibility declaration (including :**pkg** and //visibility: ones)
