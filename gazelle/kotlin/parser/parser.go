package parser

import (
	"log"
	"strings"

	Log "github.com/aspect-build/aspect-cli/pkg/logger"

	treeutils "github.com/aspect-build/aspect-cli/gazelle/common/treesitter"
)

type ParseResult struct {
	File    string
	Imports []string
	Package string
	HasMain bool
}

type Parser interface {
	Parse(filePath string, source []byte) (*ParseResult, []error)
}

type treeSitterParser struct {
	Parser
}

func NewParser() Parser {
	p := treeSitterParser{}

	return &p
}

const importsQuery = `
	(source_file
		(import_list
			(import_header
				(identifier) @from
				(wildcard_import)? @from-wild
			)
		)
	)

	(source_file
		(package_header
			(identifier) @package
		)
	)

	(source_file
		(function_declaration
			(simple_identifier) @equals-main
		)

		(#eq? @equals-main "main")
	)
`

func (p *treeSitterParser) Parse(filePath string, sourceCode []byte) (*ParseResult, []error) {
	var result = &ParseResult{
		File:    filePath,
		Imports: make([]string, 0),
	}

	errs := make([]error, 0)

	tree, err := treeutils.ParseSourceCode(treeutils.Kotlin, filePath, sourceCode)
	if err != nil {
		errs = append(errs, err)
	}

	if tree != nil {
		defer tree.Close()

		q := treeutils.GetQuery(treeutils.Kotlin, importsQuery)
		for queryResult := range tree.Query(q) {
			Log.Tracef("Kotlin AST Query %q: %v", filePath, queryResult)

			caps := queryResult.Captures()
			if from, isFrom := caps["from"]; isFrom {
				if _, isFromWild := caps["from-wild"]; !isFromWild {
					if lastDot := strings.LastIndex(from, "."); lastDot != -1 {
						from = from[:lastDot]
					}
				}
				result.Imports = append(result.Imports, from)
			} else if pkg, isPackage := caps["package"]; isPackage {
				if result.Package != "" {
					log.Fatalf("Multiple package declarations found in %q: %s and %s", filePath, result.Package, pkg)
				}

				result.Package = pkg
			} else if _, isMain := caps["equals-main"]; isMain {
				result.HasMain = true
			} else {
				log.Fatalf("Unexpected query result for %q: %v", filePath, queryResult)
			}
		}

		treeErrors := tree.QueryErrors()
		if treeErrors != nil {
			errs = append(errs, treeErrors...)
		}
	}

	return result, errs
}
