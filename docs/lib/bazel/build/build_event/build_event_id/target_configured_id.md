

## TargetConfiguredId.aspect

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetConfiguredId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">aspect</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

If empty, the id refers to the expansion of the target. If not-empty, the id refers to the expansion of an aspect applied to the (already expanded) target.

For example, when building an apple\_binary that depends on proto\_library
"//:foo\_proto", there will be two TargetConfigured events for
"//:foo\_proto":

1. An event with an empty aspect, corresponding to actions producing
   language-agnostic outputs from the proto\_library; and
2. An event with aspect "ObjcProtoAspect", corresponding to Objective-C
   code generation.

***

## TargetConfiguredId.label

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">TargetConfiguredId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">label</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>
