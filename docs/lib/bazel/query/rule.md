

`property` **Rule.attribute**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">attribute</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/bazel/query/attribute">attribute</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

All of the attributes that describe the rule.

`property` **Rule.configured\_rule\_input**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">configured_rule_input</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">configured_rule_input</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

`property` **Rule.default\_setting**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">default_setting</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The set of all "features" inherited from the rule's package declaration.

`property` **Rule.definition\_stack**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">definition_stack</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The Starlark call stack for the definition of the rule class of this particular rule instance. If empty, either populating the field was not enabled on the command line with the --proto:definition\_stack flag or the rule is a native one.

`property` **Rule.deprecated\_is\_skylark**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">deprecated_is_skylark</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

`property` **Rule.deprecated\_public\_by\_default**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">deprecated_public_by_default</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bool">bool</a></span></code></pre>

The rule's class's public by default value.

`property` **Rule.instantiation\_stack**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">instantiation_stack</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

The Starlark call stack at the moment the rule was instantiated. Each entry has the form "file:line:col: function". The outermost stack frame (" <toplevel>", the BUILD file) appears first; the frame for the rule function itself is omitted. The file name may be relative to package's source root directory.

Requires --proto:instantiation\_stack=true.

`property` **Rule.location**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">location</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></code></pre>

The BUILD file and line number of the location (formatted as \<absolute\_path>:\<line\_number>:\<column\_number>) in the rule's package's BUILD file where the rule instance was instantiated. The line number will be that of a rule invocation or macro call (that in turn invoked a rule). See <https://bazel.build/rules/macros#macro-creation>

`property` **Rule.name**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">name</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The name of the rule (formatted as an absolute label, e.g. //foo/bar:baz).

`property` **Rule.rule\_class**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">rule_class</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

The rule class name (e.g., java\_library).

Note that the rule class name may not uniquely identify a rule class, since
two different .bzl files may define different rule classes with the same
name. To uniquely identify the rule class, see rule\_class\_key field below.

`property` **Rule.rule\_class\_info**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">rule_class_info</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">rule_info</span></span></span></code></pre>

Stardoc-format rule class API definition for this rule. Includes both Starlark-defined and native (including inherited) attributes; does not include hidden or explicitly undocumented attributes.

Populated only for the first rule in the stream with a given
rule\_class\_key.

Requires --proto:rule\_classes=true

`property` **Rule.rule\_class\_key**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">rule_class_key</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></code></pre>

A key uniquely identifying the rule's rule class. Stable between repeated blaze query invocations (assuming that there are no changes to Starlark files and the same blaze binary is invoked with the same options).

Requires --proto:rule\_classes=true

`property` **Rule.rule\_input**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">rule_input</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

All of the inputs to the rule (formatted as absolute labels). These are predecessors in the dependency graph.

`property` **Rule.rule\_output**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">rule_output</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></code></pre>

All of the outputs of the rule (formatted as absolute labels). These are successors in the dependency graph.

`property` **Rule.skylark\_environment\_hash\_code**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">Rule</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">skylark_environment_hash_code</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></code></pre>

Hash encapsulating the behavior of this Starlark rule. Any change to this rule's definition that could change its behavior will be reflected here.
