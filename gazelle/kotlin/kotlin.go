package gazelle

import (
	"path"
	"strings"
)

import "github.com/emirpasic/gods/sets/treeset"

func IsNativeImport(impt string) bool {
	return strings.HasPrefix(impt, "kotlin.") || strings.HasPrefix(impt, "kotlinx.") || strings.HasPrefix(impt, "java.") || strings.HasPrefix(impt, "javax.")
}

type KotlinTarget struct {
	Imports *treeset.Set
}

/**
 * Information for kotlin library target including:
 * - kotlin files
 * - kotlin import statements from all files
 * - kotlin packages implemented
 */
type KotlinLibTarget struct {
	KotlinTarget

	Packages *treeset.Set
	Files    *treeset.Set
}

func NewKotlinLibTarget() *KotlinLibTarget {
	return &KotlinLibTarget{
		KotlinTarget: KotlinTarget{
			Imports: treeset.NewWith(importStatementComparator),
		},
		Packages: treeset.NewWithStringComparator(),
		Files:    treeset.NewWithStringComparator(),
	}
}

/**
 * Information for kotlin binary (main() method) including:
 * - kotlin import statements from all files
 * - the package
 * - the file
 */
type KotlinBinTarget struct {
	KotlinTarget

	File    string
	Package string
}

func NewKotlinBinTarget(file, pkg string) *KotlinBinTarget {
	return &KotlinBinTarget{
		KotlinTarget: KotlinTarget{
			Imports: treeset.NewWith(importStatementComparator),
		},
		File:    file,
		Package: pkg,
	}
}

// packagesKey is the name of a private attribute set on generated kt_library
// rules. This attribute contains the KotlinTarget for the target.
const packagesKey = "_kotlin_package"

func toBinaryTargetName(mainFile string) string {
	base := strings.ToLower(strings.TrimSuffix(path.Base(mainFile), path.Ext(mainFile)))

	// TODO: move target name template to directive
	return base + "_bin"
}
