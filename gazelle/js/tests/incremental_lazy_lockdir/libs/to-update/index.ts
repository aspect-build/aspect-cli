// Relative files, including json
export const version = require('./package.json').version

// Relative files within a subdirectory (containing a BUILD)
export * from "./sub"

// pnpm dep from own package.json
export const aa = require('@aspect-test/a');

// pnpm dep from a parent package.json
export const ab = require('@aspect-test/b');
// ... and a file within that package
export const abp = require('@aspect-test/b/package.json');

// pnpm workspace:* dep from own package.json
export * from "@lib/a"
// ... and a file within that package
export * from "@lib/a/package.json"

// tsconfig 'paths' based import
export * from "tsconfig-paths/b"

// tsconfig 'paths' based imports with extra odd junk
export * from "tsconfig-paths/foo/../a"
