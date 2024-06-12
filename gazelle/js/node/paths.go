package gazelle

import (
	"path"
	"strings"
)

func ParseImportPath(imp string) (string, string) {
	// Imports of local files are never packages
	if imp == "" || imp[0] == '/' || imp[0] == '.' {
		return "", imp
	}

	// Imports of npm-like packages
	// Trim to only the package name or scoped package name
	if imp[0] == '@' {
		parts := strings.SplitN(imp, "/", 3)

		// Scoped packages must have a second part, otherwise it is not a "package" import.
		if len(parts) < 2 {
			return "", imp
		}

		if len(parts) == 2 {
			return imp, ""
		}

		return path.Join(parts[0], parts[1]), parts[2]
	}

	pkgEnd := strings.Index(imp, "/")
	if pkgEnd == -1 {
		return imp, ""
	}

	return imp[:pkgEnd], imp[pkgEnd+1:]
}
