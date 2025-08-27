package plugin

import (
	common "github.com/aspect-build/aspect-cli/gazelle/common"
	godsutils "github.com/emirpasic/gods/utils"
)

type Symbol struct {
	Id       string // The unique id of the symbol
	Provider string // The provider type of the symbol
}

type TargetImport struct {
	Symbol

	// Optional imports will not be treated as resolution errors when not found.
	Optional bool

	// Where the import is from such as file path, for debugging
	From string
}

type TargetSymbol struct {
	Symbol

	// The label producing the symbol
	Label Label
}

/**
 * A bazel target declaration describing the target name/type/attributes as
 * well as symbols representing imports and exports of the target.
 */
type TargetDeclaration struct {
	Name  string
	Kind  string
	Attrs map[string]interface{}

	// Names (possibly as paths) exported from this target
	Symbols []Symbol
}

type TargetAction interface{}

type AddTargetAction struct {
	TargetAction
	TargetDeclaration
}

type RemoveTargetAction struct {
	TargetAction
	Name string
	Kind string
}

func symbolComparator(a, b interface{}) int {
	nc := godsutils.StringComparator(a.(Symbol).Id, b.(Symbol).Id)
	if nc != 0 {
		return nc
	}

	return godsutils.StringComparator(a.(Symbol).Provider, b.(Symbol).Provider)
}

func TargetImportComparator(a, b interface{}) int {
	nc := symbolComparator(a, b)
	if nc != 0 {
		return nc
	}

	return godsutils.StringComparator(a.(TargetImport).From, b.(TargetImport).From)
}

func TargetExportComparator(a, b interface{}) int {
	nc := symbolComparator(a, b)
	if nc != 0 {
		return nc
	}

	return common.LabelComparator(a.(TargetSymbol).Label, b.(TargetSymbol).Label)
}
