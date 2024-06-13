// Import just about everything for various test cases.
// Individual imports may also have test cases.

// Side-effects only imports
require('./side-effects');

// Local
const { lib } = require('./lib');

// Local with post-transpilation file extension
const { lib: lib2 } = require('./lib.js');

// Subdirectory within same BUILD including implicit index.ts import
const { subdir_index } = require('./subdir');
const { subdir_lib } = require('./subdir/lib');
const { subdir_parent_ref } = require('./subdir/parent-ref');

// Sub-project imports, including implicit index.ts import
const { subproject_index } = require('./subproject');
const { subproject_lib } = require('./subproject/lib');

// Import of a project with indirect deps
const { backref_subproject_lib } = require('./subproject-backref/lib');

// DTS
const { t } = require('./t');
const { sd } = require('./subdir/sd');
const { sp } = require('./subproject/sp');

// Type-only imports
const { Foo } = require('./types');

const fooVal: typeof Foo = 123;

console.log(
    lib,
    lib2,
    subdir_index,
    subdir_lib,
    subdir_parent_ref,
    subproject_index,
    subproject_lib,
    backref_subproject_lib,
    t,
    sd,
    sp,
    fooVal
);
