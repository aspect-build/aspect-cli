/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package root

// Global flags defined on the root cobra command
// NB: cobra only sets these after init and before run,
// so you must reference them as pointers rather than
// values before flag parsing has occurred.
var (
	Interactive bool
	CfgFile     string
)
