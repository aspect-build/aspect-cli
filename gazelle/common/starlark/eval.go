package starlark

import (
	"fmt"
	"path"

	"github.com/bazelbuild/bazel-gazelle/label"
	"go.starlark.net/lib/json"
	"go.starlark.net/starlark"
	"go.starlark.net/syntax"

	stdlib "github.com/aspect-build/aspect-cli/gazelle/common/starlark/stdlib"
)

// The signature for a starlark module loader (see starlark.Thread.Load)
type moduleLoader = func(thread *starlark.Thread, module string) (starlark.StringDict, error)

// Remain simple and strict like bazel starlark.
var opts = &syntax.FileOptions{
	TopLevelControl: true,
	GlobalReassign:  false,
}

// Copy of go.starlark.net/repl.MakeLoadOptions with the following changes:
// * Add and passthru ExecFileOptions `src interface{}, predeclared starlark.StringDict`
//
// See https://github.com/google/starlark-go/blob/0d3f41d403af5d6607cdf241f12b7e0572f2cb58/repl/repl.go#L171-L200
func makeLoadOptions(opts *syntax.FileOptions, predeclared starlark.StringDict) moduleLoader {
	type entry struct {
		globals starlark.StringDict
		err     error
	}

	var cache = make(map[string]*entry)

	return func(thread *starlark.Thread, module string) (starlark.StringDict, error) {
		e, ok := cache[module]
		if e == nil {
			if ok {
				// request for package whose loading is in progress
				return nil, fmt.Errorf("cycle in load graph")
			}

			// Add a placeholder to indicate "load in progress".
			cache[module] = nil

			// Load it.
			thread := &starlark.Thread{Name: "exec " + module, Load: thread.Load}
			globals, err := starlark.ExecFileOptions(opts, thread, module, nil, predeclared)
			e = &entry{globals, err}

			// Update the cache.
			cache[module] = e
		}
		return e.globals, e.err
	}
}

// Wrap a `moduleLoader` and add support for load()ing similar to bazel rulesets.
func createRepoLoader(rootDir string, loader moduleLoader) moduleLoader {
	return func(thread *starlark.Thread, module string) (starlark.StringDict, error) {
		moduleLabel, err := label.Parse(module)
		if err != nil {
			return nil, fmt.Errorf("invalid load() label: %s", module)
		}

		if moduleLabel.Repo != "" {
			// FUTURE: loading from external repositories, local repository by name.
			return nil, fmt.Errorf("repository load() unsupported: %s", module)
		}

		modulePath := path.Join(rootDir, moduleLabel.Pkg, moduleLabel.Name)

		return loader(thread, modulePath)
	}
}

func threadPrint(t *starlark.Thread, msg string) {
	// TODO: stdout? log?
	fmt.Printf("%s: %s\n", t.Name, msg)
}

func Eval(rootDir, starpath string, libs starlark.StringDict, locals map[string]interface{}) (starlark.StringDict, error) {
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

	loader := makeLoadOptions(opts, predeclared)
	loader = createRepoLoader(rootDir, loader)

	thread := starlark.Thread{
		Name:  "AspectConfigure",
		Load:  loader,
		Print: threadPrint,
	}
	for localName, local := range locals {
		thread.SetLocal(localName, local)
	}

	return starlark.ExecFileOptions(opts, &thread, path.Join(rootDir, starpath), nil, predeclared)
}
