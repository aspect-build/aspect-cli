package starlark

//#include "tree_sitter/parser.h"
//TSLanguage *tree_sitter_starlark();
import "C"
import (
	"unsafe"
)

func Language() unsafe.Pointer {
	return unsafe.Pointer(C.tree_sitter_starlark())
}
