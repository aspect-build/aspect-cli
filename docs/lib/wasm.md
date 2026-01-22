

## Wasm.instantiate

<pre class="language-python"><code><span class="source python"><span class="meta function python"><span class="storage type function python">def</span> <span class="entity name function python"><span class="meta generic-name python">Wasm</span></span>.<span class="entity name function python"><span class="meta generic-name python">instantiate</span></span></span><span class="meta function parameters python"><span class="punctuation section parameters begin python">(</span></span><span class="meta function parameters python">
    <span class="variable parameter python">path</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    /<span class="punctuation separator parameters python">,</span>
    *<span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">args</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">list</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="meta structure list python"><span class="punctuation section list begin python">[</span><span class="punctuation section list end python">]</span></span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">env</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">dict</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span>, <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">preopened_dirs</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">dict</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span>, <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">inherit_stdio</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/bool">bool</a><span class="punctuation definition string end python">&#39;</span></span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">True</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
    <span class="variable parameter python">imports</span></span><span class="meta function parameters annotation python"><span class="punctuation separator annotation parameter python">:</span> <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">dict</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span>, <span class="meta item-access python"><span class="meta qualified-name python"><span class="support type python">dict</span></span></span><span class="meta item-access python"><span class="punctuation section brackets begin python">[</span></span><span class="meta item-access arguments python"><span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/str">str</a><span class="punctuation definition string end python">&#39;</span></span></span>, <span class="meta qualified-name python"><span class="meta generic-name python">typing</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">Any</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span></span><span class="meta item-access python"><span class="punctuation section brackets end python">]</span></span> </span><span class="meta function parameters default-value python"><span class="keyword operator assignment python">=</span> <span class="constant language python">...</span></span><span class="meta function parameters python"><span class="punctuation separator parameters python">,</span>
<span class="punctuation section parameters end python">)</span></span><span class="meta function python"> </span><span class="meta function annotation return python"><span class="punctuation separator annotation return python">-&gt;</span> <span class="meta string python"><span class="string quoted single python"><span class="punctuation definition string begin python">&#39;</span></span></span><span class="meta string python"><span class="string quoted single python"><a href="/lib/wasm/instance">Instance</a><span class="punctuation definition string end python">&#39;</span></span></span></span></span></code></pre>

Instantiate a WASM module with optional WASI configuration and host function imports.

#### Parameters

* `path`: - Path to the WASM binary file
* `args`: - Command-line arguments to pass to the WASM module (WASI)
* `env`: - Environment variables to set for the WASM module (WASI)
* `preopened_dirs`: - Directories to pre-open for filesystem access (WASI). Keys are host paths, values are guest paths.
* `inherit_stdio`: - Whether to inherit stdin/stdout/stderr from the host (default: true)
* `imports`: - Host function imports, organized by module name and function name. **Important:** Host functions must be imported from a loaded module via `load()`, not defined in the same file. This ensures they are frozen and can be safely stored in the WASM runtime.

#### Returns

A `wasm.Instance` that can be used to call exported functions.

#### Details

# Example

```starlark
# In host_funcs.axl:
def get_term_size(ctx, memory) -> tuple[int, int]:
    return (80, 24)

# In main.axl:
load("./host_funcs.axl", "get_term_size")

instance = ctx.wasm.instantiate(
    "app.wasm",
    args = ["--verbose"],
    env = {"HOME": "/home/user"},
    preopened_dirs = {"/tmp": "/sandbox"},
    imports = {
        "env": {
            "get_term_size": get_term_size,
        }
    },
)
instance.start()  # Call _start for WASI modules
result = instance.exports.my_function(42)
```

# Host Function Signature

Host functions receive two injected arguments followed by WASM arguments:

* `ctx` - Task context (currently `None`, reserved for future use)
* `memory` - WASM memory access (currently `None`, reserved for future use)
* Additional arguments correspond to the WASM function signature

Return values are converted back to WASM types. Use tuples for multi-value returns.
