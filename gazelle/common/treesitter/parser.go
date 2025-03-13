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
	"fmt"
	"log"
	"path"

	golang "github.com/aspect-build/aspect-cli/gazelle/common/treesitter/grammars/golang"
	"github.com/aspect-build/aspect-cli/gazelle/common/treesitter/grammars/java"
	"github.com/aspect-build/aspect-cli/gazelle/common/treesitter/grammars/json"
	"github.com/aspect-build/aspect-cli/gazelle/common/treesitter/grammars/kotlin"
	"github.com/aspect-build/aspect-cli/gazelle/common/treesitter/grammars/starlark"
	"github.com/aspect-build/aspect-cli/gazelle/common/treesitter/grammars/tsx"
	"github.com/aspect-build/aspect-cli/gazelle/common/treesitter/grammars/typescript"
	sitter "github.com/smacker/go-tree-sitter"
)

type LanguageGrammar string

const (
	Kotlin      LanguageGrammar = "kotlin"
	Starlark                    = "starlark"
	Typescript                  = "typescript"
	TypescriptX                 = "tsx"
	JSON                        = "json"
	Java                        = "java"
	Go                          = "go"
)

type ASTQueryResult interface {
	Captures() map[string]string
}

type AST interface {
	Query(query TreeQuery) <-chan ASTQueryResult
	QueryErrors() []error

	// Wrapper utils
	// TODO: delete
	QueryStrings(query TreeQuery, returnVar string) []string
	RootNode() *sitter.Node

	// Release all resources related to this AST.
	// The AST is most likely no longer usable after this call.
	Close()
}
type treeAst struct {
	lang       LanguageGrammar
	filePath   string
	sourceCode []byte

	sitterTree *sitter.Tree
}

var _ AST = (*treeAst)(nil)

func (tree *treeAst) Close() {
	tree.sitterTree.Close()
	tree.sitterTree = nil
	tree.sourceCode = nil
}

func (tree *treeAst) String() string {
	return fmt.Sprintf("treeAst{\n lang: %q,\n filePath: %q,\n AST:\n  %v\n}", tree.lang, tree.filePath, tree.sitterTree.RootNode().String())
}

func toSitterLanguage(lang LanguageGrammar) *sitter.Language {
	switch lang {
	case Go:
		return golang.GetLanguage()
	case Java:
		return java.GetLanguage()
	case JSON:
		return json.GetLanguage()
	case Kotlin:
		return kotlin.GetLanguage()
	case Starlark:
		return starlark.GetLanguage()
	case Typescript:
		return typescript.GetLanguage()
	case TypescriptX:
		return tsx.GetLanguage()
	}

	log.Panicf("Unknown LanguageGrammar %q", lang)
	return nil
}

func PathToLanguage(p string) LanguageGrammar {
	return extensionToLanguage(path.Ext(p))
}

// Based on https://github.com/github-linguist/linguist/blob/master/lib/linguist/languages.yml
var EXT_LANGUAGES = map[string]LanguageGrammar{
	"go": Go,

	"kt":  Kotlin,
	"ktm": Kotlin,
	"kts": Kotlin,

	"bzl": Starlark,

	"ts":  Typescript,
	"cts": Typescript,
	"mts": Typescript,
	"js":  Typescript,
	"mjs": Typescript,
	"cjs": Typescript,

	"tsx": TypescriptX,
	"jsx": TypescriptX,

	"java": Java,
	"jav":  Java,
	"jsh":  Java,
	"json": JSON,
}

// In theory, this is a mirror of
// https://github.com/github-linguist/linguist/blob/master/lib/linguist/languages.yml
func extensionToLanguage(ext string) LanguageGrammar {
	var lang, found = EXT_LANGUAGES[ext[1:]]

	// TODO: allow override or fallback language for files
	if !found {
		log.Panicf("Unknown source file extension %q", ext)
	}

	return lang
}

func ParseSourceCode(lang LanguageGrammar, filePath string, sourceCode []byte) (AST, error) {
	ctx := context.Background()

	parser := sitter.NewParser()
	parser.SetLanguage(toSitterLanguage(lang))

	tree, err := parser.ParseCtx(ctx, nil, sourceCode)
	if err != nil {
		return nil, err
	}

	return &treeAst{lang: lang, filePath: filePath, sourceCode: sourceCode, sitterTree: tree}, nil
}
