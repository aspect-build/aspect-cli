

## ArtifactMetrics.output\_artifacts\_from\_action\_cache

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ArtifactMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">output_artifacts_from_action_cache</span></span><span class="punctuation separator annotation variable python">:</span> <span class="constant language python">None</span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/build_metrics/artifact_metrics/files_metric">files_metric</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Measures all output artifacts from actions that were cached locally via the action cache. These artifacts were already present on disk at the start of the build. Does not include Skyframe-cached actions' outputs.

***

## ArtifactMetrics.output\_artifacts\_seen

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ArtifactMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">output_artifacts_seen</span></span><span class="punctuation separator annotation variable python">:</span> <span class="constant language python">None</span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/build_metrics/artifact_metrics/files_metric">files_metric</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Measures all output artifacts from executed actions. This includes actions that were cached locally (via the action cache) or remotely (via a remote cache or executor), but does *not* include outputs of actions that were cached internally in Skyframe.

***

## ArtifactMetrics.source\_artifacts\_read

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ArtifactMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">source_artifacts_read</span></span><span class="punctuation separator annotation variable python">:</span> <span class="constant language python">None</span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/build_metrics/artifact_metrics/files_metric">files_metric</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Measures all source files newly read this build. Does not include unchanged sources on incremental builds.

***

## ArtifactMetrics.top\_level\_artifacts

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">ArtifactMetrics</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">top_level_artifacts</span></span><span class="punctuation separator annotation variable python">:</span> <span class="constant language python">None</span> <span class="keyword operator arithmetic python">|</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bazel/build/build_event/build_metrics/artifact_metrics/files_metric">files_metric</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Measures all artifacts that belong to a top-level output group. Does not deduplicate, so if there are two top-level targets in this build that share an artifact, it will be counted twice.
