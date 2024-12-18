// typeof-import in an argument
function x<T>(a: typeof import('jquery')): T {
    return a as T;
}

// typeof-import in a variable declaration
export const F: typeof import('@aspect-test/a') = null as any;

// typeof-import in a function invocation
const f = x<typeof import('@aspect-test/b')>(null);

// typeof-importi n an 'as X' expression
const g = null as any as typeof import('@aspect-test/c');

// typeof-import in another function invocation
(async function () {
    return await x<typeof import('@aspect-test/d')>();
})();

// typeof-import in a variable declaration
new Set<typeof import('@aspect-test/e')>();

// type-of in export *
export type * as Foo from '@aspect-test/f';

// type-of in import *
import type * as Bar from '@aspect-test/g';
