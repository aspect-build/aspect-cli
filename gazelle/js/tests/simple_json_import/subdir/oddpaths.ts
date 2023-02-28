// Import json with some odd relative paths.
// May not cause new bazel deps[] but the odd paths must be resolved.

export { key as key1 } from '../data.json';
export { key as key2 } from '../subdir/data.json';
export { n as key3 } from '../subproject/./data.json';
export { key as key4 } from '././././data.json';
