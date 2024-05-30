package json

//#include "src/tree_sitter/parser.h"
//TSLanguage *tree_sitter_json();
import "C"
import (
	"unsafe"

	sitter "github.com/smacker/go-tree-sitter"
)

func GetLanguage() *sitter.Language {
	ptr := unsafe.Pointer(C.tree_sitter_json())
	return sitter.NewLanguage(ptr)
}
