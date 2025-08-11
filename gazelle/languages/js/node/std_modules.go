package gazelle

import (
	"strings"

	"github.com/emirpasic/gods/sets/treeset"
)

var nativeModulesSet = createNativeModulesSet()

func createNativeModulesSet() *treeset.Set {
	set := treeset.NewWithStringComparator()

	for _, m := range NativeModules {
		set.Add(m)
	}

	return set
}

func IsNodeImport(imprt string) bool {
	return strings.HasPrefix(imprt, "node:") || nativeModulesSet.Contains(imprt)
}
