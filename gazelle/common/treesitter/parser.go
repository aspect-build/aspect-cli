/*
 * Copyright 2023 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

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
