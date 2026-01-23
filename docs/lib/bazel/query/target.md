

## Target.environment\_group

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Target</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">environment_group</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/query/environment_group">environment_group</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

An environment group.

***

## Target.generated\_file

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Target</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">generated_file</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/query/generated_file">generated_file</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

A generated file that is the output of a rule.

***

## Target.package\_group

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Target</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">package_group</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/query/package_group">package_group</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

A package group.

***

## Target.rule

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Target</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">rule</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/query/rule">rule</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

If this target represents a rule, the rule is stored here.

***

## Target.source\_file

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Target</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">source_file</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/none">None</a><span class="punctuation definition string end python">&#39;</span></span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/query/source_file">source_file</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

A file that is not generated by the build system (version controlled or created by the test harness).

***

## Target.type

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Target</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">type</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The type of target contained in the message.
