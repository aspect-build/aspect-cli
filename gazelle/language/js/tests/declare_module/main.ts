import { a } from 'lib-a';
import { foo } from 'lib-lib';
import { bar } from 'https://url.mod'

export const x = foo + bar;

a();

// Dynamic imports to verify both syntaxes
(async function() {
    await import('lib-a')
    await import('lib-lib')
    await import('https://url.mod')
}())
