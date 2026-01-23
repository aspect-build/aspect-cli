

`property` **RunfilesTree.empty\_files**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">empty_files</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The paths of empty files relative to the subdirectory of the runfiles tree root corresponding to the main repository.

`property` **RunfilesTree.input\_set\_id**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">input_set_id</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

The entry ID of the set of artifacts in the runfiles tree that are symlinked at their canonical locations relative to the tree path. See SpawnLogReconstructor#getRunfilesPaths for how to recover the tree-relative paths of the artifacts from their exec paths.

In case of path collisions, later artifacts overwrite earlier ones and
artifacts override custom symlinks.

The referenced set must not transitively contain any runfile trees.

`property` **RunfilesTree.legacy\_external\_runfiles**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">legacy_external_runfiles</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

Whether the runfiles tree contains external runfiles at their legacy locations (e.g. \_main/external/bazel\_tools/tools/bash/runfiles.bash) in addition to the default locations (e.g. bazel\_tools/tools/bash/runfiles.bash).

`property` **RunfilesTree.path**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">path</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The runfiles tree path.

`property` **RunfilesTree.repo\_mapping\_manifest**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">repo_mapping_manifest</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/build_event/file">file</a></span></code></pre>

The "\_repo\_mapping" file at the root of the runfiles tree, if it exists. Only the digest is stored as the relative path is fixed.

`property` **RunfilesTree.root\_symlinks\_id**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">root_symlinks_id</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

The entry ID of the set of symlink entries with paths relative to the root of the runfiles tree.

`property` **RunfilesTree.symlinks\_id**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">RunfilesTree</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">symlinks_id</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

The entry ID of the set of symlink entries with paths relative to the subdirectory of the runfiles tree root corresponding to the main repository.
