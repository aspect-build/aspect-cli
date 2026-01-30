

`function` **Env.arch**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Env</span></span>.<span class="entity name function python"><span class="meta generic-name python">arch</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/str">str</a></span></span></code></pre>

Returns the CPU architecture.

Returns a string describing the CPU architecture, such as
"x86\_64", "aarch64", etc.

`function` **Env.aspect\_cli\_version**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Env</span></span>.<span class="entity name function python"><span class="meta generic-name python">aspect_cli_version</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/str">str</a></span></span></code></pre>

Returns the version of the Aspect CLI.

`function` **Env.current\_dir**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Env</span></span>.<span class="entity name function python"><span class="meta generic-name python">current_dir</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/str">str</a></span></span></code></pre>

Returns the current working directory as a path.

**Platform**-specific behavior

This function currently corresponds to the `getcwd` function on Unix
and the `GetCurrentDirectoryW` function on Windows.

**Errors**

Fails if the current working directory value is invalid.
Possible cases:

* Current directory does not exist.
* There are insufficient permissions to access the current directory.

`function` **Env.current\_exe**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Env</span></span>.<span class="entity name function python"><span class="meta generic-name python">current_exe</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/str">str</a></span></span></code></pre>

`function` **Env.home\_dir**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Env</span></span>.<span class="entity name function python"><span class="meta generic-name python">home_dir</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></span></code></pre>

Returns the path of the current user's home directory if known.

This may return `None` if getting the directory fails or if the platform does not have user home directories.

For storing user data and configuration it is often preferable to use more specific directories.
For example, [XDG Base Directories] on Unix or the `LOCALAPPDATA` and `APPDATA` environment variables on Windows.

[XDG Base Directories]: https://specifications.freedesktop.org/basedir-spec/latest/

**Unix**

* Returns the value of the 'HOME' environment variable if it is set
  (including to an empty string).
* Otherwise, it tries to determine the home directory by invoking the `getpwuid_r` function
  using the UID of the current user. An empty home directory field returned from the
  `getpwuid_r` function is considered to be a valid value.
* Returns `None` if the current user has no entry in the /etc/passwd file.

**Windows**

* Returns the value of the 'USERPROFILE' environment variable if it is set, and is not an empty string.
* Otherwise, [`GetUserProfileDirectory`][msdn] is used to return the path. This may change in the future.

[msdn]: https://docs.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-getuserprofiledirectorya

In UWP (Universal Windows Platform) targets this function is unimplemented and always returns `None`.

`function` **Env.os**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Env</span></span>.<span class="entity name function python"><span class="meta generic-name python">os</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/str">str</a></span></span></code></pre>

Returns the operating system name.

Returns a string describing the operating system in use, such as
"linux", "macos", "windows", etc.

`function` **Env.root\_dir**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Env</span></span>.<span class="entity name function python"><span class="meta generic-name python">root_dir</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/str">str</a></span></span></code></pre>

Returns the project root directory.

This project root directory is found starting at current working directory and searching upwards
through its ancestors for repository boundary marker files (such as `MODULE.aspect`, `MODULE.bazel`,
`MODULE.bazel.lock`, `REPO.bazel`, `WORKSPACE`, or `WORKSPACE.bazel`). The first ancestor directory
containing any of these files is considered the project root. If no such directory is found, the
current directory is used as the project root.

`function` **Env.temp\_dir**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Env</span></span>.<span class="entity name function python"><span class="meta generic-name python">temp_dir</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/str">str</a></span></span></code></pre>

Returns the path of a temporary directory.

The temporary directory may be shared among users, or between processes
with different privileges; thus, the creation of any files or directories
in the temporary directory must use a secure method to create a uniquely
named file. Creating a file or directory with a fixed or predictable name
may result in "insecure temporary file" security vulnerabilities. Consider
using a crate that securely creates temporary files or directories.

Note that the returned value may be a symbolic link, not a directory.

**Platform**-specific behavior

On Unix, returns the value of the `TMPDIR` environment variable if it is
set, otherwise the value is OS-specific:

* On Darwin-based OSes (macOS, iOS, etc) it returns the directory provided
  by `confstr(_CS_DARWIN_USER_TEMP_DIR, ...)`, as recommended by [Apple's
  security guidelines][appledoc].
* On all other unix-based OSes, it returns `/tmp`.

On Windows, the behavior is equivalent to that of [`GetTempPath2`][GetTempPath2] /
[`GetTempPath`][GetTempPath], which this function uses internally.

[GetTempPath2]: https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-gettemppath2a

[GetTempPath]: https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-gettemppatha

[appledoc]: https://developer.apple.com/library/archive/documentation/Security/Conceptual/SecureCodingGuide/Articles/RaceConditions.html#//apple_ref/doc/uid/TP40002585-SW10

`function` **Env.var**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Env</span></span>.<span class="entity name function python"><span class="meta generic-name python">var</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">key</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <a href="/lib/str">str</a></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <a href="/lib/str">str</a></span></span></code></pre>

Fetches the environment variable key from the current process.

`function` **Env.vars**

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Env</span></span>.<span class="entity name function python"><span class="meta generic-name python">vars</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <a href="/lib/list">list</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/tuple">tuple</a><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><a href="/lib/str">str</a>, <span class="constant language python">...</span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

Returns an iterator of (variable, value) pairs of strings, for all the environment variables of the current process.

The returned iterator contains a snapshot of the process's environment
variables at the time of this invocation. Modifications to environment
variables afterwards will not be reflected in the returned iterator.
