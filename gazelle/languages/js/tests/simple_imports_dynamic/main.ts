// Import just about everything for various test cases.
// Individual imports may also have test cases.

// Local
import('./lib').then(console.log);

// Subdirectory within same BUILD including implicit index.ts import
import('./subdir').then(console.log);
import('./subdir/lib').then(console.log);
import('./subdir/parent-ref').then(console.log);

// Sub-project imports, including implicit index.ts import
import('./subproject').then(console.log);
import('./subproject/lib').then(console.log);

// Import of a project with indirect deps
import('./subproject-backref/lib').then(console.log);

// Imports of various file extensions
import('./exts/common-dts.cjs').then(console.log);
import('./exts/common-ts.cjs').then(console.log);
import('./exts/module-ts.mjs').then(console.log);
import('./exts/module-dts.mjs').then(console.log);
import('./exts/either-ts').then(console.log);
import('./exts/either-ts.js').then(console.log);
import('./exts/either-dts').then(console.log);
import('./exts/either-dts.js').then(console.log);
import('./exts/both-ts.cjs').then(console.log);
import('./exts/both-ts.mjs').then(console.log);
import('./exts/both-ts.js').then(console.log);
import('./exts/both-ts').then(console.log);
import('./exts/both-dts.cjs').then(console.log);
import('./exts/both-dts.mjs').then(console.log);
import('./exts/both-dts.js').then(console.log);
import('./exts/both-dts').then(console.log);
