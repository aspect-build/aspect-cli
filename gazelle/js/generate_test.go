package gazelle

import (
	"fmt"
	"path"
	"reflect"
	"sort"
	"testing"
)

func TestGenerate(t *testing.T) {
	for _, tc := range []struct {
		pkg, from, impt string
		expected        string
	}{
		// Simple
		{
			pkg:      "",
			from:     "from.ts",
			impt:     "./empty",
			expected: "empty",
		},
		{
			pkg:      "",
			from:     "from/sub.ts",
			impt:     "./empty",
			expected: "from/empty",
		},
		{
			pkg:      "foo",
			from:     "from.ts",
			impt:     "./bar",
			expected: "foo/bar",
		},
		{
			pkg:      "foo",
			from:     "from/sub.ts",
			impt:     "./bar",
			expected: "foo/from/bar",
		},
		// Absolute
		{
			pkg:      "",
			from:     "from.ts",
			impt:     "workspace/is/common",
			expected: "workspace/is/common",
		},
		{
			pkg:      "dont-use-me",
			from:     "from.ts",
			impt:     "workspace/is/common",
			expected: "workspace/is/common",
		},
		// Parent (..)
		{
			pkg:      "",
			from:     "from.ts",
			impt:     "./foo/../bar",
			expected: "bar",
		},
		{
			pkg:      "",
			from:     "from/sub.ts",
			impt:     "./foo/../bar",
			expected: "from/bar",
		},
		{
			pkg:      "foo",
			from:     "from.ts",
			impt:     "../bar",
			expected: "bar",
		},
		{
			pkg:      "foo",
			from:     "from/sub.ts",
			impt:     "../bar",
			expected: "foo/bar",
		},
		{
			pkg:      "foo",
			from:     "from.ts",
			impt:     "./baz/../bar",
			expected: "foo/bar",
		},
		{
			pkg:      "foo",
			from:     "from/sub.ts",
			impt:     "./baz/../bar",
			expected: "foo/from/bar",
		},
		// Absolute parent
		{
			pkg:      "dont-use-me",
			from:     "from.ts",
			impt:     "baz/../bar",
			expected: "bar",
		},
		{
			pkg:      "dont-use-me",
			from:     "from/sub.ts",
			impt:     "baz/../bar",
			expected: "bar",
		},
		// URLs
		{
			pkg:      "dont-use-me",
			from:     "anywhere.ts",
			impt:     "https://me.com",
			expected: "https://me.com",
		},
		{
			pkg:      "dont-use-me",
			from:     "anywhere.ts",
			impt:     "http://me.com",
			expected: "http://me.com",
		},
		{
			pkg:      "dont-use-me",
			from:     "anywhere.ts",
			impt:     "anything://me",
			expected: "anything://me",
		},
	} {
		desc := fmt.Sprintf("toImportSpecPath(%s, %s, %s)", tc.pkg, tc.from, tc.impt)

		t.Run(desc, func(t *testing.T) {
			importPath := toImportSpecPath(path.Join(tc.pkg, tc.from), tc.impt)

			if !reflect.DeepEqual(importPath, tc.expected) {
				t.Errorf("toImportSpecPath('%s', '%s', '%s'): \nactual:   %s\nexpected:  %s\n", tc.pkg, tc.from, tc.impt, importPath, tc.expected)
			}
		})
	}

	t.Run("toImportPaths", func(t *testing.T) {
		// Traditional [.d].ts[x] don't require an extension
		assertImports(t, "bar.ts", []string{"bar", "bar.js"})
		assertImports(t, "bar.tsx", []string{"bar", "bar.js"})
		assertImports(t, "bar.d.ts", []string{"bar", "bar.js"})
		assertImports(t, "foo/bar.ts", []string{"foo/bar", "foo/bar.js"})
		assertImports(t, "foo/bar.tsx", []string{"foo/bar", "foo/bar.js"})
		assertImports(t, "foo/bar.d.ts", []string{"foo/bar", "foo/bar.js"})

		// Traditional [.d].ts[x] index files
		assertImports(t, "bar/index.ts", []string{"bar/index", "bar/index.js", "bar"})
		assertImports(t, "bar/index.d.ts", []string{"bar/index", "bar/index.js", "bar"})
		assertImports(t, "bar/index.tsx", []string{"bar/index", "bar/index.js", "bar"})

		// .mjs and .cjs files require an extension
		assertImports(t, "bar.mts", []string{"bar.mjs"})
		assertImports(t, "bar/index.mts", []string{"bar/index.mjs", "bar"})
		assertImports(t, "bar.d.mts", []string{"bar.mjs"})
		assertImports(t, "bar.cts", []string{"bar.cjs"})
		assertImports(t, "bar/index.cts", []string{"bar/index.cjs", "bar"})
		assertImports(t, "bar.d.cts", []string{"bar.cjs"})
	})
}

func assertImports(t *testing.T, p string, expected []string) {
	actual := toImportPaths(p)

	// Order doesn't matter so sort to ignore order
	sort.Strings(actual)
	sort.Strings(expected)

	if !reflect.DeepEqual(actual, expected) {
		t.Errorf("toImportPaths('%s'): \nactual:   %s\nexpected:  %s\n", p, actual, expected)
	}
}
