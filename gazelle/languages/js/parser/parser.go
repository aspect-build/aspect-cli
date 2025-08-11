package parser

import (
	"log"
	"path"
	"regexp"
	"strings"

	Log "github.com/aspect-build/aspect-cli/gazelle/common/logger"

	treeutils "github.com/aspect-build/aspect-cli/gazelle/common/treesitter"
)

// Parse and find imports using TreeSitter (https://tree-sitter.github.io/).
// ESM imports which are always at the root of the AST can be easily found.
// CommonJS and dynamic imports can be anywhere within the AST and are found using
// TreeSitter AST queries.

// TreeSitter playground: https://tree-sitter.github.io/tree-sitter/playground

type ParseResult struct {
	Imports []string
	Modules []string
}

type ParseErrors struct {
	Errors []error
}

var _ error = (*ParseErrors)(nil)

func (pe *ParseErrors) Error() string {
	s := make([]string, 0, len(pe.Errors))
	for _, err := range pe.Errors {
		s = append(s, err.Error())
	}
	return strings.Join(s, "\n")
}

// A query finding dependencies and declarations in TypeScript/JavaScript files.
//
// Query matches may include captures:
// - from: a string representing an imported resource such as a name or path
// - triple-slash: a triple-slash directive comment
// - defined: a string representing a defined module name
const importsQuery = `
	(call_expression
		function: [
			(identifier) @equals-require
			(import)
		]
		arguments: (arguments . (comment)* . (string (string_fragment) @from))

		(#eq? @equals-require "require")
	)

	(program
		(import_statement
			source: (string (string_fragment) @from)
		)
	)

	(program
		(export_statement
			source: (string (string_fragment) @from)
		)
	)

	(program
		(comment) @triple-slash
		(#match? @triple-slash "^///\\s*<reference\\s+(?:path|types)\\s*=")
	)

	(program
		(ambient_declaration
			(module
				body: (statement_block [
					(import_statement
						source: (string (string_fragment) @from)
					)
					(export_statement
						source: (string (string_fragment) @from)
					)
				])
			)
		)
	)

	(program
		(ambient_declaration
			(module
				name: (string (string_fragment) @defined)
			)
		)
	)
`

// Note that we intentionally omit "lib" here since these directives do not result in a separate dependency
// See: https://www.typescriptlang.org/docs/handbook/triple-slash-directives.html#-reference-lib-
// > This directive allows a file to explicitly include an existing built-in lib file
var tripleSlashRe = regexp.MustCompile(`^///\s*<reference\s+(?:path|types)\s*=\s*"(?P<lib>[^"]+)"`)

func ParseSource(filePath string, sourceCode []byte) (ParseResult, error) {
	var imports []string
	var modules []string
	var errs []error

	lang := filenameToLanguage(filePath)

	// Parse the source code
	tree, err := treeutils.ParseSourceCode(lang, filePath, sourceCode)
	if err != nil {
		errs = append(errs, err)
	}

	if tree != nil {
		defer tree.Close()

		// Query for more complex non-root node imports.
		q := treeutils.GetQuery(lang, importsQuery)
		for queryResult := range tree.Query(q) {
			Log.Tracef("AST Query %q: %v", filePath, queryResult)

			caps := queryResult.Captures()
			if from, isFrom := caps["from"]; isFrom {
				imports = append(imports, from)
			} else if tripSlash, isTripSlash := caps["triple-slash"]; isTripSlash {
				// Parse triple-slash directives
				if lib, ok := getTripleSlashDirectiveModule(tripSlash); ok {
					imports = append(imports, lib)
				}
			} else if defined, isDefined := caps["defined"]; isDefined {
				modules = append(modules, defined)
			} else {
				log.Fatalf("Unexpected query result for %q: %v", filePath, queryResult)
			}
		}

		// Parse errors. Only log them due to many false positives potentially caused by issues
		// such as only parsing a single file at a time so type information from other files is missing.
		if Log.IsLevelEnabled(Log.TraceLevel) {
			treeErrors := tree.QueryErrors()
			if treeErrors != nil {
				Log.Tracef("TreeSitter query errors: %v", treeErrors)
			}
		}
	}

	result := ParseResult{
		Imports: imports,
		Modules: modules,
	}

	var perr error
	if len(errs) > 0 {
		perr = &ParseErrors{errs}
	}

	return result, perr
}

// Extract the module name out of a triple-slash directive comment.
//
// See: https://www.typescriptlang.org/docs/handbook/triple-slash-directives.html
func getTripleSlashDirectiveModule(comment string) (string, bool) {
	submatches := tripleSlashRe.FindAllStringSubmatchIndex(comment, -1)
	if len(submatches) != 1 {
		return "", false
	}

	lib := tripleSlashRe.ExpandString(make([]byte, 0), "$lib", comment, submatches[0])
	return string(lib), len(lib) > 0
}

// File extension to language key
func filenameToLanguage(filename string) treeutils.LanguageGrammar {
	ext := path.Ext(filename)[1:]
	switch ext {
	case "tsx":
		return treeutils.TypescriptX
	case "jsx":
		return treeutils.TypescriptX
	}

	return treeutils.Typescript
}
