package gazelle

import "testing"

func TestParsePackage(t *testing.T) {
	t.Run("ParseImportPath", func(t *testing.T) {
		// Non-package imports
		assertParseImport("/x", "", "/x", t)
		assertParseImport("/x/", "", "/x/", t)
		assertParseImport("./x", "", "./x", t)
		assertParseImport(".", "", ".", t)
		assertParseImport("..", "", "..", t)
		assertParseImport("../f", "", "../f", t)
		assertParseImport("", "", "", t)

		// Package imports
		assertParseImport("x", "x", "", t)
		assertParseImport("x/", "x", "", t)
		assertParseImport("x/y", "x", "y", t)
		assertParseImport("x-y", "x-y", "", t)
		assertParseImport("x/y/z", "x", "y/z", t)

		// Scoped package imports
		assertParseImport("@scope/package", "@scope/package", "", t)
		assertParseImport("@scope/package/", "@scope/package", "", t)
		assertParseImport("@scope/package/subpackage", "@scope/package", "subpackage", t)
		assertParseImport("@scope/package/subpackage/", "@scope/package", "subpackage/", t)
		assertParseImport("@scope/package/subpackage/b", "@scope/package", "subpackage/b", t)
	})
}

func assertParseImport(imp, expectedPkg, expectedPath string, t *testing.T) {
	pkg, p := ParseImportPath(imp)
	if pkg != expectedPkg || p != expectedPath {
		t.Errorf("parse(%s): expected [%q, %q] got [%q, %q]", imp, expectedPkg, expectedPath, pkg, p)
	}
}

func TestTypesPackage(t *testing.T) {
	t.Run("ParseImportPath", func(t *testing.T) {
		assertTypesPackage("x", "@types/x", t)
		assertTypesPackage("x-y", "@types/x-y", t)
		assertTypesPackage("@scope/package", "@types/scope__package", t)
	})
}

func assertTypesPackage(imp, expectedPkg string, t *testing.T) {
	pkg := ToAtTypesPackage(imp)
	if pkg != expectedPkg {
		t.Errorf("@types(%s): expected %q got %q", imp, expectedPkg, pkg)
	}
}
