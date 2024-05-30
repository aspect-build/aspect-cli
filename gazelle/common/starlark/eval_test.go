package starlark

import (
	"os"
	"path"
	"testing"

	"go.starlark.net/starlark"
)

func run(t *testing.T, code string) (starlark.StringDict, error) {
	testFile := path.Join(os.TempDir(), "test.star")

	f, err := os.Create(testFile)
	if err != nil {
		t.Errorf("Temp star create failure: %v", err)
	}

	_, err = f.WriteString(code)
	if err != nil {
		t.Errorf("Temp star write failure: %v", err)
	}

	err = f.Close()
	if err != nil {
		t.Errorf("Temp star close failure: %v", err)
	}

	return Eval(testFile, make(map[string]starlark.Value), make(map[string]interface{}))
}

func runOk(t *testing.T, code string) starlark.StringDict {
	v, err := run(t, code)
	if err != nil {
		t.Error(err)
	}
	return v
}

func TestStarlarkEval(t *testing.T) {
	t.Run("basic", func(t *testing.T) {
		res := runOk(t, "1")
		if len(res) != 0 {
			t.Errorf("Expected empty result, got %v", res)
		}
	})

	t.Run("basic vars", func(t *testing.T) {
		res := runOk(t, `
x = 1
y = "s"
`)

		x, hasX := res["x"]
		if !hasX {
			t.Errorf("Expected x to be defined")
		}

		if x.Type() != "int" {
			t.Errorf("Expected type int %v", x.Type())
		}

		i, intErr := starlark.AsInt32(x)
		if intErr != nil {
			t.Errorf("Expected int %v", intErr)
		}

		if i != 1 {
			t.Errorf("Expected 1, got %v", i)
		}

		y := res["y"]
		if y.Type() != "string" {
			t.Errorf("Expected type string %v", y.Type())
		}
		ystr, _ := starlark.AsString(y)
		if ystr != "s" {
			t.Errorf("Expected string 's', got %v", ystr)
		}
	})

	t.Run("basic func", func(t *testing.T) {
		res := runOk(t, `
def f():
	return 1
`)
		f, hasF := res["f"]
		if !hasF {
			t.Errorf("Expected f to be defined")
		}

		if f.Type() != "function" {
			t.Errorf("Expected type function %v", f.Type())
		}

		v, err := Call(f, starlark.Tuple{}, nil)
		if err != nil {
			t.Errorf("Expected call to succeed %v", err)
		}

		i, intErr := starlark.AsInt32(v)
		if intErr != nil {
			t.Errorf("Expected int %v", intErr)
		}

		if i != 1 {
			t.Errorf("Expected 1, got %v", i)
		}
	})
}
