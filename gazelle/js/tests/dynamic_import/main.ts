// Nested dynamic
function foo() {
    return import(`${window.globalVar}/abc`);
}

// Valid with various quotes
import("./lib")
import('./lib')
import(`./lib`)

// Dynamic requires
import("foo" + "bar")
import(`${window.foo}`)

// Various invalid imports
import(3 * 5)
import(3, "not-first")

// A valid first arg
import("lib", 3)

// Various invalid cjs requires
require(3 * 5)
require(3, "not-first")

// Valid first arg
require("lib", 3)
