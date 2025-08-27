package gazelle

import (
	"strings"
)

func ParseImportPath(imp string) (string, string) {
	// Imports of local files are never packages
	if imp == "" || imp[0] == '/' || imp[0] == '.' {
		return "", imp
	}

	// Imports of @scoped-package-like paths
	if imp[0] == '@' {
		scopeEnd := strings.IndexByte(imp, '/')
		if scopeEnd == -1 {
			return "", imp
		}
		subPkg := imp[scopeEnd+1:]
		subPkgEnd := strings.IndexByte(subPkg, '/')
		if subPkgEnd == -1 {
			return imp, ""
		}
		return imp[:scopeEnd+subPkgEnd+1], imp[scopeEnd+subPkgEnd+2:]
	}

	// Imports of package-like paths
	pkgEnd := strings.IndexByte(imp, '/')
	if pkgEnd == -1 {
		return imp, ""
	}

	return imp[:pkgEnd], imp[pkgEnd+1:]
}

func ToAtTypesPackage(pkg string) string {
	// @scoped packages
	if pkg[0] == '@' {
		if i := strings.IndexRune(pkg, '/'); i != -1 {
			return "@types/" + pkg[1:i] + "__" + pkg[i+1:]
		}
		return ""
	}

	// packages with trailing 0
	if i := strings.IndexRune(pkg, '/'); i != -1 {
		return "@types/" + pkg[:i]
	}

	return "@types/" + pkg
}
