package parser

import (
	"testing"
)

var testCases = []struct {
	desc, kt string
	// Specify a filename so esbuild knows how to load the file.
	filename string
	pkg      string
	imports  []string
}{
	{
		desc:     "empty",
		kt:       "",
		filename: "empty.kt",
		pkg:      "",
		imports:  []string{},
	},
	{
		desc: "simple",
		kt: `
import a.B
import c.D as E
	`,
		filename: "simple.kt",
		pkg:      "",
		imports:  []string{"a", "c"},
	},
	{
		desc: "stars",
		kt: `package a.b.c

import  d.y.* 
		`,
		filename: "stars.kt",
		pkg:      "a.b.c",
		imports:  []string{"d.y"},
	},
	{
		desc: "comments",
		kt: `
/*dlfkj*/package /*dlfkj*/ x // x
//z
import a.B // y
//z

/* asdf */ import /* asdf */ c./* asdf */D // w
import /* fdsa */ d/* asdf */.* // w
				`,
		filename: "comments.kt",
		pkg:      "x",
		imports:  []string{"a", "c", "d"},
	},
}

func TestTreesitterParser(t *testing.T) {

	for _, tc := range testCases {
		t.Run(tc.desc, func(t *testing.T) {
			actualImports, _ := NewParser().ParseImports(tc.filename, tc.kt)

			if !equal(actualImports, tc.imports) {
				t.Errorf("Imports...\nactual:  %#v;\nexpected: %#v\nkotlin code:\n%v", actualImports, tc.imports, tc.kt)
			}

			actualPackage, _ := NewParser().ParsePackage(tc.filename, tc.kt)

			if actualPackage != tc.pkg {
				t.Errorf("Package....\nactual:  %#v;\nexpected: %#v\nkotlin code:\n%v", actualPackage, tc.pkg, tc.kt)
			}
		})
	}
}

func equal[T comparable](a, b []T) bool {
	if len(a) != len(b) {
		return false
	}
	for i, v := range a {
		if v != b[i] {
			return false
		}
	}
	return true
}
