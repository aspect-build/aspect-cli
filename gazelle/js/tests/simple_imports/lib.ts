export const lib = 'lib';
export * from './subdir';
export * from './subproject';
export * from './subproject-backref';
export * as i1 from './subproject-index';
export * as i2 from './subproject-index/index';
export * from './t';

// Imports of various file extensions
export * from './exts/common-dts.cjs';
export * from './exts/common-ts.cjs';
export * from './exts/module-ts.mjs';
export * from './exts/module-dts.mjs';
export * as e1 from './exts/either-ts';
export * as e2 from './exts/either-ts.js';
export * as t1 from './exts/either-dts';
export * as t2 from './exts/either-dts.js';
export * as b1 from './exts/both-ts.cjs';
export * as b2 from './exts/both-ts.mjs';
export * as b3 from './exts/both-ts.js';
export * as b4 from './exts/both-ts';
export * as btsx from './exts/both-tsx';
export * as b5 from './exts/both-dts.cjs';
export * as b6 from './exts/both-dts.mjs';
export * as b7 from './exts/both-dts.js';
export * as b8 from './exts/both-dts';
