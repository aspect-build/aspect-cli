

## RunfilesTree.empty\_files

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">empty_files</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">list</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The paths of empty files relative to the subdirectory of the runfiles tree root corresponding to the main repository.

***

## RunfilesTree.input\_set\_id

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">input_set_id</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The entry ID of the set of artifacts in the runfiles tree that are symlinked at their canonical locations relative to the tree path. See SpawnLogReconstructor#getRunfilesPaths for how to recover the tree-relative paths of the artifacts from their exec paths.

In case of path collisions, later artifacts overwrite earlier ones and
artifacts override custom symlinks.

The referenced set must not transitively contain any runfile trees.

***

## RunfilesTree.path

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">path</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The runfiles tree path.

***

## RunfilesTree.repo\_mapping\_manifest

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">repo_mapping_manifest</span></span><span class="punctuation separator annotation variable python">:</span> <span class="constant language python">None</span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/execution_log/exec_log_entry/file">file</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The "\_repo\_mapping" file at the root of the runfiles tree, if it exists. Only the digest is stored as the relative path is fixed.

***

## RunfilesTree.root\_symlinks\_id

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">root_symlinks_id</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The entry ID of the set of symlink entries with paths relative to the root of the runfiles tree.

***

## RunfilesTree.symlinks\_id

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">symlinks_id</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

The entry ID of the set of symlink entries with paths relative to the subdirectory of the runfiles tree root corresponding to the main repository.
