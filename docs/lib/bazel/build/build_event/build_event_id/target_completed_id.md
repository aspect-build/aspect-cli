

`property` **TargetCompletedId.aspect**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetCompletedId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">aspect</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>

If empty, the id refers to the completion of the target. If not-empty, the id refers to the completion of an aspect applied to the (already completed) target.

For example, when building an apple\_binary that depends on proto\_library
"//:foo\_proto", there will be two TargetCompleted events for
"//:foo\_proto":

1. An event with an empty aspect, corresponding to actions producing
   language-agnostic outputs from the proto\_library; and
2. An event with aspect "ObjcProtoAspect", corresponding to Objective-C
   code generation.

`property` **TargetCompletedId.configuration**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetCompletedId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">configuration</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">configuration_id</span></span></span></code></pre>

The configuration for which the target was built.

`property` **TargetCompletedId.label**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetCompletedId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">label</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib/str">str</a></span></code></pre>
