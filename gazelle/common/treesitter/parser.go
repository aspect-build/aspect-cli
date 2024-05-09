package treesitter

import (
	"context"
	"log"

	"aspect.build/cli/gazelle/common/treesitter/grammars/kotlin"
	"aspect.build/cli/gazelle/common/treesitter/grammars/tsx"
	"aspect.build/cli/gazelle/common/treesitter/grammars/typescript"
	sitter "github.com/smacker/go-tree-sitter"
)

type LanguageGrammar = int

const (
	Kotlin LanguageGrammar = iota
	Typescript
	TypescriptX
)

type AST interface {
	QueryStrings(query, returnVar string) []string

	QueryErrors() []error
}
type TreeAst struct {
	AST

	lang       LanguageGrammar
	filePath   string
	sourceCode []byte

	// TODO: don't make public
	SitterTree *sitter.Tree
}

func toSitterLanguage(lang LanguageGrammar) *sitter.Language {
	switch lang {
	case Kotlin:
		return kotlin.GetLanguage()
	case Typescript:
		return typescript.GetLanguage()
	case TypescriptX:
		return tsx.GetLanguage()
	}

	log.Fatalf("Unknown LanguageGrammar %q", lang)
	return nil
}

func ParseSourceCode(lang LanguageGrammar, filePath string, sourceCode []byte) (AST, error) {
	ctx := context.Background()

	parser := sitter.NewParser()
	parser.SetLanguage(toSitterLanguage(lang))

	tree, err := parser.ParseCtx(ctx, nil, sourceCode)
	if err != nil {
		return nil, err
	}

	return TreeAst{lang: lang, filePath: filePath, sourceCode: sourceCode, SitterTree: tree}, nil
}
