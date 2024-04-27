import { A as AliasA } from 'new-alias-a';
import { A } from 'new-star/a';
import { B } from 'new-star/b';
import { C } from 'src/overlib/o';

console.log(A, B, AliasA, C);
