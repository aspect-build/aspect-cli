package starlark

import (
	"os"
	"path"
	"testing"

	"go.starlark.net/starlark"
)

func run(t *testing.T, code string) (starlark.StringDict, error) {
	testDir := os.TempDir()
	testFile := "test.star"

	f, err := os.Create(path.Join(testDir, testFile))
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

	return Eval(testDir, testFile, make(map[string]starlark.Value), make(map[string]interface{}))
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
}
