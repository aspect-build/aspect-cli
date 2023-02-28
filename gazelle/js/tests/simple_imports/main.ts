// Import just about everything for various test cases.
// Individual imports may also have test cases.

// Local
import { lib } from './lib';

// Subdirectory within same BUILD including implicit index.ts import
import { subdir_index } from './subdir';
import { subdir_lib } from './subdir/lib';
import { subdir_parent_ref } from './subdir/parent-ref';

// Sub-project imports, including implicit index.ts import
import { subproject_index } from './subproject';
import { subproject_lib } from './subproject/lib';

// Import of a project with indirect deps
import { backref_subproject_lib } from './subproject-backref/lib';

console.log(
    lib,
    subdir_index,
    subdir_lib,
    subdir_parent_ref,
    subproject_index,
    subproject_lib,
    backref_subproject_lib
);
