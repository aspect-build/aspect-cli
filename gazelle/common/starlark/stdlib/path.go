package starlark

import (
	"path"

	utils "github.com/aspect-build/aspect-cli/gazelle/common/starlark/utils"

	"go.starlark.net/starlark"
)

func path_base(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var s starlark.Value
	if err := starlark.UnpackPositionalArgs(b.Name(), args, kwargs, 1, &s); err != nil {
		return nil, err
	}

	return starlark.String(path.Base(s.(starlark.String).GoString())), nil
}

func path_dirname(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var s starlark.Value
	if err := starlark.UnpackPositionalArgs(b.Name(), args, kwargs, 1, &s); err != nil {
		return nil, err
	}

	return starlark.String(path.Dir(s.(starlark.String).GoString())), nil
}

func path_ext(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var s starlark.Value
	if err := starlark.UnpackPositionalArgs(b.Name(), args, kwargs, 1, &s); err != nil {
		return nil, err
	}

	return starlark.String(path.Ext(s.(starlark.String).GoString())), nil
}

func path_join(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, _ []starlark.Tuple) (starlark.Value, error) {
	return starlark.String(path.Join(utils.ReadStringTuple(args)...)), nil
}

var Path = utils.CreateModule("path", map[string]utils.ModuleFunction{
	"base":    path_base,
	"dirname": path_dirname,
	"ext":     path_ext,
	"join":    path_join,
}, make(map[string]starlark.Value))
