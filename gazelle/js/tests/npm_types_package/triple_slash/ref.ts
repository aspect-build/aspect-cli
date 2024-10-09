/// References to npm packages, via both "lib" and "types"
/// <reference lib="jquery">
/// <reference types="@testing-library/jest-dom">

// Transpiled .ts files, referenced both with and without the .d.ts extension.
// IDEs flag this as an error that should be a regular `import` but this is still valid.
/// <reference path="../transpiled/types">
/// <reference path="../transpiled/types.d.ts">

// ... same with .d.ts source files.
/// <reference path="../only_types/types">
/// <reference path="../only_types/types.d.ts">

// ... same with source files with the same BUILD.
/// <reference path="./defs">
/// <reference path="./defs.d.ts">
