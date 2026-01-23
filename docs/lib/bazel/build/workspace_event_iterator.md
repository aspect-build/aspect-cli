

`function` **WorkspaceEventIterator.done**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">WorkspaceEventIterator</span></span>.<span class="entity name function python"><span class="meta generic-name python">done</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/bool">bool</a></span></span></code></pre>

Returns `True` if stream is complete and all the events are received via `for` or calling `pop` repeatedly.

`function` **WorkspaceEventIterator.try\_pop**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">WorkspaceEventIterator</span></span>.<span class="entity name function python"><span class="meta generic-name python">try_pop</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/bazel/build/workspace_event/workspace_event">workspace_event</a></span></span></code></pre>

Returns `WorkspaceEvent` if event buffer is not empty. Maximum `1000` events is buffered at once.
