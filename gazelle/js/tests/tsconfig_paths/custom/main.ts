import { A } from 'star/a';
import { B } from 'star/b';
import { A as AliasA } from 'alias-a';
import { A as RootDotA } from 'lib/a';
import { C1 } from 'multi-c/c1';
import { C2 } from 'multi-c/c2';
import { F } from 'f1';
import { Test } from '@/nested/test';

console.log(A, B, AliasA, RootDotA, C1, C2, F, Test);
