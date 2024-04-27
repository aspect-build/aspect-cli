import { Test } from '@/nested/test';
import { A as BadlyA } from '@badly-scoped-a';
import { A as ScopedA } from '@scoped/a';
import { A as AliasA } from 'alias-a';
import { F } from 'f1';
import { A as RootDotA } from 'lib/a';
import { C1 } from 'multi-c/c1';
import { C2 } from 'multi-c/c2';
import { A } from 'star/a';
import { B } from 'star/b';

console.log(A, B, AliasA, BadlyA, ScopedA, RootDotA, C1, C2, F, Test);
