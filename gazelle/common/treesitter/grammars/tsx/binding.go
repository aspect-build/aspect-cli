package tsx

//#include "tsx/src/tree_sitter/parser.h"
//TSLanguage *tree_sitter_tsx();
import "C"
import (
	"unsafe"

	sitter "github.com/smacker/go-tree-sitter"
)

func GetLanguage() *sitter.Language {
	ptr := unsafe.Pointer(C.tree_sitter_tsx())
	return sitter.NewLanguage(ptr)
}
