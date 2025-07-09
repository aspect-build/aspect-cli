package parser

import (
	"testing"
)

var testCases = []struct {
	desc, kt string
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

/* asdf */ import /* asdf */ c.D // w
import /* fdsa */ d/* asdf */.* // w
				`,
		filename: "comments.kt",
		pkg:      "x",
		imports:  []string{"a", "c", "d"},
	},
	// Value classes: https://github.com/fwcd/tree-sitter-kotlin/commit/80834a15154448cfa795bfa6b8be3559af1753fc
	{
		desc: "value-classes",
		kt: `
@JvmInline
value class Password(private val s: String)
	`,
		filename: "simple.kt",
		pkg:      "",
		imports:  []string{},
	},
}

func TestTreesitterParser(t *testing.T) {
	for _, tc := range testCases {
		t.Run(tc.desc, func(t *testing.T) {
			res, errs := NewParser().Parse(tc.filename, []byte(tc.kt))
			if len(errs) > 0 {
				t.Errorf("Errors parsing %q: %v", tc.filename, errs)
			}

			if !equal(res.Imports, tc.imports) {
				t.Errorf("Imports...\nactual:  %#v;\nexpected: %#v\nkotlin code:\n%v", res.Imports, tc.imports, tc.kt)
			}

			if res.Package != tc.pkg {
				t.Errorf("Package....\nactual:  %#v;\nexpected: %#v\nkotlin code:\n%v", res.Package, tc.pkg, tc.kt)
			}
		})
	}

	t.Run("main detection", func(t *testing.T) {
		res, errs := NewParser().Parse("main.kt", []byte("fun main() {}"))
		if len(errs) > 0 {
			t.Errorf("Parse error: %v", errs)
		}

		if !res.HasMain {
			t.Errorf("main method should be detected")
		}

		res, errs = NewParser().Parse("x.kt", []byte(`
package my.demo
fun main() {}
		`))
		if len(errs) > 0 {
			t.Errorf("Parse error: %v", errs)
		}
		if !res.HasMain {
			t.Errorf("main method should be detected with package")
		}

		res, errs = NewParser().Parse("x.kt", []byte(`
package my.demo
import kotlin.text.*
fun main() {}
		`))
		if len(errs) > 0 {
			t.Errorf("Parse error: %v", errs)
		}
		if !res.HasMain {
			t.Errorf("main method should be detected with imports")
		}
	})
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
