package starlark

import (
	"fmt"

	"go.starlark.net/lib/json"
	"go.starlark.net/repl"
	"go.starlark.net/starlark"
	"go.starlark.net/syntax"

	stdlib "github.com/aspect-build/aspect-cli/gazelle/common/starlark/stdlib"
)

// Remain simple and strict like bazel starlark.
var opts = &syntax.FileOptions{
	TopLevelControl: true,
	GlobalReassign:  false,
}

func threadPrint(t *starlark.Thread, msg string) {
	// TODO: stdout? log?
	fmt.Printf("%s: %s\n", t.Name, msg)
}

var threadTemplate = starlark.Thread{
	Name:  "AspectConfigure",
	Load:  repl.MakeLoadOptions(opts),
	Print: threadPrint,
}

func Eval(starpath string, libs starlark.StringDict, locals map[string]interface{}) (starlark.StringDict, error) {
	// Predeclared libs in addition to the go.starlark.net/starlark standard library:
	// * https://github.com/google/starlark-go/blob/f86470692795f8abcf9f837a3c53cf031c5a3d7e/starlark/library.go#L36-L73
	// * https://github.com/google/starlark-go/blob/f86470692795f8abcf9f837a3c53cf031c5a3d7e/cmd/starlark/starlark.go#L96-L100
	predeclared := starlark.StringDict{
		"path": stdlib.Path,
		"json": json.Module,
	}

	for libName, lib := range libs {
		predeclared[libName] = lib
	}

	thread := threadTemplate
	for localName, local := range locals {
		thread.SetLocal(localName, local)
	}

	return starlark.ExecFileOptions(opts, &thread, starpath, nil, predeclared)
}

func Call(c starlark.Value, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	thread := threadTemplate

	return starlark.Call(&thread, c.(starlark.Callable), args, kwargs)
}
