// Imports of various syntaxes.
// Each odd syntax should trigger a ts_project(dep).

import /* comment */ "../exts/common-dts.cjs";
export /* comment */ * /* comment */  from /* comment */"../subproject";
import( /* comment */ "../subdir" /* comment */ )
import /* comment */ (
    // comment
    "../subproject-backref/lib"
)
