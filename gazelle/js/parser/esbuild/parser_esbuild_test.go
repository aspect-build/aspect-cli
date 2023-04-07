package gazelle

import (
	"testing"

	"aspect.build/cli/gazelle/js/parser/tests"
)

func TestEsbuildParser(t *testing.T) {
	tests.RunParserTests(t, NewParser(), false, "esbuild")
}
