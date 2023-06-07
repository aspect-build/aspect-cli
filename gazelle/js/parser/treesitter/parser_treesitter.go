package gazelle

import (
	"context"
	"path"
	"strings"

	"github.com/sirupsen/logrus"
	sitter "github.com/smacker/go-tree-sitter"
	"github.com/smacker/go-tree-sitter/typescript/tsx"
	"github.com/smacker/go-tree-sitter/typescript/typescript"

	treeutils "aspect.build/cli/gazelle/common/treesitter"
	"aspect.build/cli/gazelle/js/parser"
)

var Log = logrus.New()

// Parse and find imports using TreeSitter (https://tree-sitter.github.io/).
// ESM imports which are always at the root of the AST can be easily found.
// CommonJS and dynamic imports can be anywhere within the AST and are found using
// TreeSitter AST queries.

// TreeSitter playground: https://tree-sitter.github.io/tree-sitter/playground

type TreeSitterParser struct {
	parser.Parser

	parser *sitter.Parser
}

func NewParser() parser.Parser {
	p := TreeSitterParser{
		parser: sitter.NewParser(),
	}

	return &p
}

// Queries finding import statements, tagging such Nodes as 'from' captures.
// Optionally filtering captures using 'equals-{name}' vars and #eq? statements.
var ImportQueries = []string{
	// Dynamic `import("...")` statement
	`
		(call_expression
			function: (import)
			arguments: (
				arguments (
					string (string_fragment) @from
				)
			)
		)
	`,

	// CJS `require("...")` statement
	`
		(call_expression
			function: (identifier) @equals-require
			arguments: (
				arguments (
					string (string_fragment) @from
				)
			)

			(#eq? @equals-require "require")
		)
	`,
}

// Supported languages by key
var Languages = map[string]*sitter.Language{
	"tsx":        tsx.GetLanguage(),
	"typescript": typescript.GetLanguage(),
}

func (p *TreeSitterParser) ParseImports(filePath, sourceCodeStr string) ([]string, []error) {
	imports := make([]string, 0, 5)
	errs := make([]error, 0)

	sourceCode := []byte(sourceCodeStr)
	sourceLangName := filenameToLanguage(filePath)
	sourceLang := Languages[sourceLangName]

	// Parse the source code
	tree, err := p.parseTypeScript(sourceLang, sourceCode)
	if err != nil {
		errs = append(errs, err)
	}

	if tree != nil {
		rootNode := tree.RootNode()

		// Quick pass over root nodes to find top level imports.
		for i := 0; i < int(rootNode.NamedChildCount()); i++ {
			rootImport := getRootImport(rootNode.NamedChild(i), sourceCode)

			if rootImport != nil {
				imports = append(imports, rootImport.Content(sourceCode))
			}
		}

		// Extra queries for harder to find imports.
		for _, q := range ImportQueries {
			queryResults := queryImports(treeutils.ParseQuery(sourceLangName, sourceLang, q), sourceCode, rootNode)

			imports = append(imports, queryResults...)
		}

		treeErrors := treeutils.QueryErrors(sourceLangName, sourceLang, sourceCode, rootNode)
		if treeErrors != nil {
			errs = append(errs, treeErrors...)
		}
	}

	return imports, errs
}

// Return a Node representing the `from` value of an import statement within the given root node.
func getRootImport(node *sitter.Node, sourceCode []byte) *sitter.Node {
	nodeType := node.Type()

	// Top level `import ... from "..."` statement.
	// Top level `export ... from "..."` statement.
	if nodeType == "import_statement" || nodeType == "export_statement" {
		from := node.ChildByFieldName("source")
		if from != nil {
			return from.Child(1)
		}
		return nil
	}

	return nil
}

func (p *TreeSitterParser) parseTypeScript(lang *sitter.Language, sourceCode []byte) (*sitter.Tree, error) {
	ctx := context.Background()

	p.parser.SetLanguage(lang)

	return p.parser.ParseCtx(ctx, nil, sourceCode)
}

// Run a query finding import query matches.
func queryImports(query *sitter.Query, sourceCode []byte, rootNode *sitter.Node) []string {
	imports := make([]string, 0, 5)

	// Execute the import query.
	qc := sitter.NewQueryCursor()
	defer qc.Close()
	qc.Exec(query, rootNode)

	// Collect import statements from the query results.
	for {
		m, ok := qc.NextMatch()
		if !ok {
			break
		}

		from := readFromQueryMatch(query, m, sourceCode)
		if from != nil {
			imports = append(imports, from.Node.Content(sourceCode))
		}
	}

	return imports
}

// Find and read the `from` QueryCapture from a QueryMatch.
// Filter matches based on captures value using "equals-{name}" vars.
func readFromQueryMatch(query *sitter.Query, m *sitter.QueryMatch, sourceCode []byte) *sitter.QueryCapture {
	var from *sitter.QueryCapture

	for ci, c := range m.Captures {
		cn := query.CaptureNameForId(uint32(ci))

		// Filters where a capture must equal a specific value.
		if strings.HasPrefix(cn, "equals-") {
			if c.Node.Content(sourceCode) != cn[len("equals-"):] {
				Log.Tracef("Skipping query match because %q != %q", c.Node.Content(sourceCode), cn[len("equals-"):])
				return nil
			}
			continue
		}

		if cn == "from" {
			Log.Tracef("Found import query 'from' %q", c.Node.Content(sourceCode))
			from = &c
		}
	}

	// Should never happen. All queries should have a `from` capture.
	if from == nil {
		Log.Fatalf("No import query 'from' found in query %q", query)
		return nil
	}

	return from
}

// File extension to language key
func filenameToLanguage(filename string) string {
	ext := path.Ext(filename)[1:]
	switch ext {
	case "tsx":
		return "tsx"
	case "jsx":
		return "tsx"
	}

	return "typescript"
}
