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
			res, _ := NewParser().Parse(tc.filename, tc.kt)

			if !equal(res.Imports, tc.imports) {
				t.Errorf("Imports...\nactual:  %#v;\nexpected: %#v\nkotlin code:\n%v", res.Imports, tc.imports, tc.kt)
			}

			if res.Package != tc.pkg {
				t.Errorf("Package....\nactual:  %#v;\nexpected: %#v\nkotlin code:\n%v", res.Package, tc.pkg, tc.kt)
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
