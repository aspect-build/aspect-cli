

## WorkerPoolStats.alive\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerPoolStats</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">alive_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Number of workers alive at the end of the build.

***

## WorkerPoolStats.created\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerPoolStats</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">created_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Number of workers created during a build.

***

## WorkerPoolStats.destroyed\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerPoolStats</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">destroyed_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Number of workers destroyed during a build (sum of all workers destroyed by eviction, UserExecException, IoException, InterruptedException and unknown reasons below).

***

## WorkerPoolStats.evicted\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerPoolStats</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">evicted_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Number of workers evicted during a build.

***

## WorkerPoolStats.hash

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerPoolStats</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">hash</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Hash of worker pool these stats are for. Contains information about startup flags.

***

## WorkerPoolStats.interrupted\_exception\_destroyed\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerPoolStats</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">interrupted_exception_destroyed_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Number of workers destroyed due to InterruptedExceptions.

***

## WorkerPoolStats.io\_exception\_destroyed\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerPoolStats</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">io_exception_destroyed_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Number of workers destroyed due to IoExceptions.

***

## WorkerPoolStats.mnemonic

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerPoolStats</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">mnemonic</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Mnemonic of workers these stats are for.

***

## WorkerPoolStats.unknown\_destroyed\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerPoolStats</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">unknown_destroyed_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Number of workers destroyed due to an unknown reason.

***

## WorkerPoolStats.user\_exec\_exception\_destroyed\_count

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">WorkerPoolStats</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">user_exec_exception_destroyed_count</span></span><span class="punctuation separator annotation variable python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></code></pre>

Number of workers destroyed due to UserExecExceptions.
