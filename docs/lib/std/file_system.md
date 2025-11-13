

## FileSystem.copy

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">copy</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="invalid illegal name python">from</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">to</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/int">int</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Copies the contents of one file to another. This function will also copy the permission bits of the original file to the destination file.

This function will overwrite the contents of to.
Note that if from and to both point to the same file, then the file will likely get truncated by this operation.
On success, the total number of bytes copied is returned and it is equal to the length of the to file as reported by metadata.

This function will return an error in the following situations, but is not limited to just these cases:

* from is neither a regular file nor a symlink to a regular file.
* from does not exist.
* The current process does not have the permission rights to read from or write to.
* The parent directory of to doesn’t exist.

***

## FileSystem.create\_dir

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">create_dir</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

Creates a new, empty directory at the provided path.

This function will return an error in the following situations, but is not limited to just these cases:

* User lacks permissions to create directory at path.
* A parent of the given path doesn’t exist. (To create a directory and all its missing parents at the same time, use the create&#x69;dir&#x6C;all function.)
* path already exists.

NOTE: If a parent of the given path doesn’t exist, this function will return an error. To create a directory and all its missing parents at the same time, use the create&#x69;dir&#x6C;all function.

***

## FileSystem.create\_dir\_all

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">create_dir_all</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

Recursively create a directory and all of its parent components if they are missing.

This function is not atomic. If it returns an error, any parent components it was able to create will remain.
If the empty path is passed to this function, it always succeeds without creating any directories.

The function will return an error if any directory specified in path does not exist and could not be created. There may be other error conditions; see create\_dir for specifics.

***

## FileSystem.exists

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">exists</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Returns `true` if the path points at an existing entity.

This function will traverse symbolic links to query information about the
destination file. In case of broken symbolic links this will return `false`.

Note that while this avoids some pitfalls of the `exists()` method, it still can not
prevent time-of-check to time-of-use (TOCTOU) bugs. You should only use it in scenarios
where those bugs are not an issue.

***

## FileSystem.hard\_link

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">hard_link</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">original</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">link</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

Creates a new hard link on the filesystem.

The link path will be a link pointing to the original path. Note that systems often require these two paths to both be located on the same filesystem.

If original names a symbolic link, it is platform-specific whether the symbolic link is followed. On platforms where it’s possible to not follow it, it is not followed, and the created hard link points to the symbolic link itself.

This function will return an error in the following situations, but is not limited to just these cases:

* The original path is not a file or doesn’t exist.
* The ‘link’ path already exists.

***

## FileSystem.is\_dir

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">is_dir</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Returns true if this path is for a directory.

This function will return an error in the following situations, but is not limited to just these cases:

* The user lacks permissions to perform metadata call on path.
* path does not exist.

***

## FileSystem.is\_file

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">is_file</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Returns true if this path is for a regular file.

This function will return an error in the following situations, but is not limited to just these cases:

* The user lacks permissions to perform metadata call on path.
* path does not exist.

***

## FileSystem.metadata

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">metadata</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta qualified-name python"><span class="meta generic-name python">fs</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Metadata</span></span></span></span></code></pre>

Returns the metadata about the given file or directory.

This function will return an error in the following situations, but is not limited to just these cases:

* The user lacks permissions to perform metadata call on path.
* path does not exist.

***

## FileSystem.read\_dir

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">read_dir</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">list</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta qualified-name python"><span class="meta generic-name python">fs</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">DirEntry</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span></span></code></pre>

Returns an iterator over the entries within a directory.

This function will return an error in the following situations, but is not limited to just these cases:

* The provided path doesn’t exist.
* The process lacks permissions to view the contents.
* The path points at a non-directory file.

***

## FileSystem.read\_link

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">read_link</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Reads a symbolic link, returning the file that the link points to.

This function will return an error in the following situations, but is not limited to just these cases:

* path is not a symbolic link.
* path does not exist.

***

## FileSystem.read\_to\_string

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">read_to_string</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Reads the entire contents of a file into a string.

This function will return an error under a number of different circumstances. Some of these error conditions are:

* The specified file does not exist.
* The user lacks permission to get the specified access rights for the file.
* The user lacks permission to open one of the directory components of the specified path.

***

## FileSystem.remove\_dir

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">remove_dir</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

Removes an empty directory.

If you want to remove a directory that is not empty, as well as all
of its contents recursively, consider using remove\_dir\_all instead.

This function will return an error in the following situations, but is not limited to just these cases:

* path doesn’t exist.
* path isn’t a directory.
* The user lacks permissions to remove the directory at the provided path.
* The directory isn’t empty.

***

## FileSystem.remove\_dir\_all

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">remove_dir_all</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

Removes a directory at this path, after removing all its contents. Use carefully!

This function does not follow symbolic links and it will simply remove the symbolic link itself.

See remove\_file and remove\_dir for possible errors.

***

## FileSystem.remove\_file

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">remove_file</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python"><span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span> /<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

Removes a file from the filesystem.

Note that there is no guarantee that the file is immediately deleted
(e.g., depending on platform, other open file descriptors may prevent immediate removal).

This function will return an error in the following situations, but is not limited to just these cases:

* path points to a directory.
* The file doesn’t exist.
* The user lacks permissions to remove the file.

***

## FileSystem.rename

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">rename</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="invalid illegal name python">from</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">to</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

Renames a file or directory to a new name, replacing the original file if to already exists.

This will not work if the new name is on a different mount point.

This function will return an error in the following situations, but is not limited to just these cases:

* from does not exist.
* The user lacks permissions to view contents.
* from and to are on separate filesystems.

***

## FileSystem.symlink\_metadata

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">symlink_metadata</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta qualified-name python"><span class="meta generic-name python">fs</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Metadata</span></span></span></span></code></pre>

Queries the metadata about a file without following symlinks.

This function will return an error in the following situations, but is not limited to just these cases:

* The user lacks permissions to perform metadata call on path.
* path does not exist.

***

## FileSystem.write

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">FileSystem</span></span>.<span class="entity name function python"><span class="meta generic-name python">write</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">content</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="constant language python">None</span></span></span></code></pre>

Writes a string as the entire contents of a file.

This function will create a file if it does not exist, and will entirely replace its contents if it does.
Depending on the platform, this function may fail if the full directory path does not exist.
This is a convenience function for using fs.create and \[write\_all] with fewer imports.
