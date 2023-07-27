package gazelle

import "strings"
import "github.com/emirpasic/gods/sets/treeset"

func IsNativeImport(impt string) bool {
	return strings.HasPrefix(impt, "kotlin.") || strings.HasPrefix(impt, "kotlinx.") || strings.HasPrefix(impt, "java.") || strings.HasPrefix(impt, "javax.")
}

/**
 * Information for kotlin library target including:
 * - BUILD package name
 * - kotlin import statements from all files
 * - kotlin packages implemented
 * - kotlin files with main() methods
 */
type KotlinTarget struct {
	Name string

	Imports  *treeset.Set
	Packages *treeset.Set

	Mains *treeset.Set

	Files *treeset.Set
}

func NewKotlinTarget() *KotlinTarget {
	return &KotlinTarget{
		Imports:  treeset.NewWith(importStatementComparator),
		Packages: treeset.NewWithStringComparator(),
		Mains:    treeset.NewWithStringComparator(),
		Files:    treeset.NewWithStringComparator(),
	}
}

// packagesKey is the name of a private attribute set on generated kt_library
// rules. This attribute contains the KotlinTarget for the target.
const packagesKey = "_java_packages"
