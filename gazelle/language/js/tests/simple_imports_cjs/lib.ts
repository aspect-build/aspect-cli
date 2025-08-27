exports.lib = 'lib';
exports.subdir = require('./subdir');
exports.subproject = require('./subproject');
exports.subproject_backref = require('./subproject-backref/lib');
exports.subproject_index = require('./subproject-index');
exports.subproject_index_index = require('./subproject-index/index');

// Imports of various file extensions
exports.ext_cjs = require('./exts/common-ts.cjs');
exports.ext_mjs = require('./exts/module-ts.mjs');
exports.ext_e1 = require('./exts/either-ts');
exports.ext_e2 = require('./exts/either-ts.js');
exports.ext_b1 = require('./exts/both-ts.cjs');
exports.ext_b2 = require('./exts/both-ts.mjs');
exports.ext_b3 = require('./exts/both-ts.js');
exports.ext_b4 = require('./exts/both-ts');
