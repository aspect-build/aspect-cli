package gazelle

import (
	"github.com/bazelbuild/bazel-gazelle/resolve"
	godsutils "github.com/emirpasic/gods/utils"
)

// TODO: drop? still used?
type ImportStatement struct {
	resolve.ImportSpec

	// The path of the file containing the import
	SourcePath string
}

// importStatementComparator compares modules by name.
func importStatementComparator(a, b interface{}) int {
	return godsutils.StringComparator(a.(ImportStatement).Imp, b.(ImportStatement).Imp)
}
