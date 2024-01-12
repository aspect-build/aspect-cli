package treesitter

import (
	"context"
	"log"

	sitter "github.com/smacker/go-tree-sitter"
	"github.com/smacker/go-tree-sitter/kotlin"
	"github.com/smacker/go-tree-sitter/typescript/tsx"
	"github.com/smacker/go-tree-sitter/typescript/typescript"
)

type Language = int

const (
	Kotlin Language = iota
	Typescript
	TypescriptX
)

// TODO: cache?
func getSitterLanguage(lang Language) *sitter.Language {
	switch lang {
	case Kotlin:
		return kotlin.GetLanguage()
	case Typescript:
		return typescript.GetLanguage()
	case TypescriptX:
		return tsx.GetLanguage()
	}

	log.Fatalf("Unknown Language %q", lang)
	return nil
}

func ParseSourceCode(lang Language, sourceCode []byte) (*sitter.Tree, error) {
	ctx := context.Background()

	// TODO: cache?
	parser := sitter.NewParser()
	parser.SetLanguage(getSitterLanguage(lang))

	return parser.ParseCtx(ctx, nil, sourceCode)
}
