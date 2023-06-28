package gazelle

import "strings"

func IsNativeImport(impt string) bool {
	return strings.HasPrefix(impt, "kotlin.") || strings.HasPrefix(impt, "kotlinx.") || strings.HasPrefix(impt, "java.") || strings.HasPrefix(impt, "javax.")
}
