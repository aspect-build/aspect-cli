// Various import syntaxes, each from a different BUILD to test each.

// Nothing
require('../side-effects');

// const =
const s1 = require('../exts/both-ts');
// const = with post-transpilation file extension
const s1a = require('../exts/both-ts.js');

// var = .sub
var s2 = require('../subproject-index').subproject_index;

// let { single }
let { subproject_index } = require('../subproject');

// const { single: renamed }
const { subproject_lib: foo } = require(/* comment */  '../subproject/lib' /* comment */, "ignore-this-one" );

// const { multi }
const {
    backref_subproject_index,
    backref_subproject_lib,
} = require( /* comment */ '../subproject-backref/lib');

// avoid unused-var warnings
console.log(
    s1,
    s1a,
    subproject_index,
    foo,
    backref_subproject_index,
    backref_subproject_lib
);
