package gazelle

import (
	"testing"

	"aspect.build/cli/gazelle/js/parser/tests"
)

func TestTreesitterParser(t *testing.T) {
	tests.RunParserTests(t, NewParser(), true, "treesitter")
}
