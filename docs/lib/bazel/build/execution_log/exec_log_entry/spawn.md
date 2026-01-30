

`property` **Spawn.args**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">args</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The command line arguments.

`property` **Spawn.cache\_hit**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">cache_hit</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

See SpawnExec.cache\_hit.

`property` **Spawn.cacheable**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">cacheable</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

See SpawnExec.cacheable.

`property` **Spawn.digest**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">digest</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">digest</span></span></span></code></pre>

See SpawnExec.digest. The hash function name is omitted. It can be obtained from Invocation. Unset if the file is empty.

`property` **Spawn.env\_vars**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">env_vars</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">environment_variable</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The environment variables.

`property` **Spawn.exit\_code**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">exit_code</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

See SpawnExec.exit\_code.

`property` **Spawn.input\_set\_id**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">input_set_id</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

Entry ID of the set of inputs. Unset means empty.

`property` **Spawn.metrics**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">metrics</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">spawn_metrics</span></span></span></code></pre>

See SpawnExec.metrics.

`property` **Spawn.mnemonic**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">mnemonic</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

See SpawnExec.mnemonic.

`property` **Spawn.outputs**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">outputs</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/bazel/build/execution_log/exec_log_entry/output">output</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The set of outputs.

`property` **Spawn.platform**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">platform</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/execution_log/platform">platform</a></span></code></pre>

The execution platform.

`property` **Spawn.remotable**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">remotable</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

See SpawnExec.remotable.

`property` **Spawn.remote\_cacheable**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">remote_cacheable</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/bool">bool</a></span></code></pre>

See SpawnExec.remote\_cacheable.

`property` **Spawn.runner**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">runner</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

See SpawnExec.runner.

`property` **Spawn.status**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">status</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

See SpawnExec.status.

`property` **Spawn.target\_label**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">target_label</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

See SpawnExec.label.

`property` **Spawn.timeout\_millis**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">timeout_millis</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

See SpawnExec.timeout\_millis.

`property` **Spawn.tool\_set\_id**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Spawn</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">tool_set_id</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/int">int</a></span></code></pre>

Entry ID of the subset of inputs that are tools. Unset means empty.
