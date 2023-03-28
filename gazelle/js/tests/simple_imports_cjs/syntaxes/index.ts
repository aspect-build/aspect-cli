// Various import syntaxes, each from a different BUILD to test each.

// Nothing
require('../side-effects');

// const =
const s1 = require('../exts/both-ts');

// var = .sub
var s2 = require('../subproject-index').subproject_index;

// let { single }
let { subproject_index } = require('../subproject');

// const { single: renamed }
const { subproject_lib: foo } = require('../subproject/lib');

// const { multi }
const {
    backref_subproject_index,
    backref_subproject_lib,
} = require('../subproject-backref/lib');

// avoid unused-var warnings
console.log(
    s1,
    subproject_index,
    foo,
    backref_subproject_index,
    backref_subproject_lib
);
