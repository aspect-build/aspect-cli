// Import just about everything for various test cases.
// Individual imports may also have test cases.

// Side-effects only imports
import './side-effects';
// Local
import { lib } from './lib';
// Subdirectory within same BUILD including implicit index.ts import
import { subdir_index } from './subdir';
import { subdir_lib } from './subdir/lib';
import { subdir_parent_ref } from './subdir/parent-ref';
import { sd } from './subdir/sd';
// Sub-project imports, including implicit index.ts import
import { subproject_index } from './subproject';
// Import of a project with indirect deps
import { backref_subproject_lib } from './subproject-backref/lib';
import { subproject_lib } from './subproject/lib';
import { sp } from './subproject/sp';
// DTS
import { t } from './t';
// Type-only imports
import { Foo } from './types';

const fooVal: Foo = 123;

console.log(
    lib,
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
