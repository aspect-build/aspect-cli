package starlark

import (
	"fmt"

	"go.starlark.net/starlark"
	"go.starlark.net/starlarkstruct"
)

type ModuleFunction = func(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, _ []starlark.Tuple) (starlark.Value, error)

func CreateModule(name string, funcs map[string]ModuleFunction, props map[string]starlark.Value) *starlarkstruct.Module {
	var builtins = starlark.StringDict{}
	for k, v := range funcs {
		builtins[k] = starlark.NewBuiltin(fmt.Sprintf("%s.%s", name, k), v)
	}
	for k, v := range props {
		builtins[k] = v
	}

	return &starlarkstruct.Module{
		Name:    name,
		Members: builtins,
	}
}
