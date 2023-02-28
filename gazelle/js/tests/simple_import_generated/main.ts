import { a } from './a';
import { index } from './subdir';
import { b } from './subdir/b';
import { c } from './subbuild/c';
import { d } from './subbuild-disabled/d';

console.log(a, b, c, d, index);
