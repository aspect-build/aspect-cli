import { A } from 'star/a';
import { B } from 'star/b';
import { A as AliasA } from 'alias-a';
import { A as BadlyA } from '@badly-scoped-a';
import { A as ScopedA } from '@scoped/a';
import { A as RootDotA } from 'lib/a';
import { C1 } from 'multi-c/c1';
import { C2 } from 'multi-c/c2';
import { F as Fallback1 } from 'f1';
import { F2 as FallbackSubdir } from 'f2/a';
import { F2 as FallbackRoot } from 'fallback/f2/a';

console.log(
    A,
    B,
    AliasA,
    BadlyA,
    ScopedA,
    RootDotA,
    C1,
    C2,
    Fallback1,
    FallbackSubdir,
    FallbackRoot
);
