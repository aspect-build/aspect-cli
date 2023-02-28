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
