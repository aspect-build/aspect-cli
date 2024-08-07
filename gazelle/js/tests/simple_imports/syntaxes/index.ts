// Various import syntaxes, each from a different BUILD to test each.

// Nothing
import '../side-effects';
// * as (import)
import * as s1 from '../exts/both-ts';
// * as (import) with post-transpilation file extension
import * as s1a from '../exts/both-ts.js';
// { single }
import { subproject_index } from '../subproject';
// { multi }
import {
    backref_subproject_index,
    backref_subproject_lib,
} from '../subproject-backref/lib';
// { single as renamed }
import { subproject_lib as foo } from '../subproject/lib';

// * as (export)
export * as s2 from '../subproject-index';

// avoid unused-var warnings
console.log(
    s1,
    s1a,
    subproject_index,
    foo,
    backref_subproject_index,
    backref_subproject_lib
);
