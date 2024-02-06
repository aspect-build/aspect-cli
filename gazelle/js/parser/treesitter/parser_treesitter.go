package gazelle

import (
	"path"

	Log "aspect.build/cli/pkg/logger"
	sitter "github.com/smacker/go-tree-sitter"

	treeutils "aspect.build/cli/gazelle/common/treesitter"
	"aspect.build/cli/gazelle/js/parser"
)

// Parse and find imports using TreeSitter (https://tree-sitter.github.io/).
// ESM imports which are always at the root of the AST can be easily found.
// CommonJS and dynamic imports can be anywhere within the AST and are found using
// TreeSitter AST queries.

// TreeSitter playground: https://tree-sitter.github.io/tree-sitter/playground

type TreeSitterParser struct {
	parser.Parser
}

func NewParser() parser.Parser {
	return &TreeSitterParser{}
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

func (p *TreeSitterParser) ParseSource(filePath, sourceCodeStr string) (parser.ParseResult, []error) {
	imports := make([]string, 0, 5)
	modules := make([]string, 0)
	errs := make([]error, 0)

	sourceCode := []byte(sourceCodeStr)
	lang := filenameToLanguage(filePath)

	// Parse the source code
	tree, err := treeutils.ParseSourceCode(lang, sourceCode)
	if err != nil {
		errs = append(errs, err)
	}

	if tree != nil {
		rootNode := tree.RootNode()

		// Quick pass over root nodes to find top level imports and modules
		for i := 0; i < int(rootNode.NamedChildCount()); i++ {
			node := rootNode.NamedChild(i)

			if isImportStatement(node) {
				if rootImport := getImportStatementName(node); rootImport != nil {
					imports = append(imports, rootImport.Content(sourceCode))
				}
			} else if isModuleDeclaration(node) {
				if modDeclName := getModuleDeclarationName(node); modDeclName != nil {
					modules = append(modules, modDeclName.Content(sourceCode))
				}

				// Import/export statements within a module declaration.
				if moduleRootNode := treeutils.GetNodeChildByTypePath(node, "module", "statement_block"); moduleRootNode != nil {
					for j := 0; j < int(moduleRootNode.NamedChildCount()); j++ {
						moduleNode := moduleRootNode.NamedChild(j)

						if isImportStatement(moduleNode) {
							if moduleImport := getImportStatementName(moduleNode); moduleImport != nil {
								imports = append(imports, moduleImport.Content(sourceCode))
							}
						}
					}
				}
			}
		}

		// Extra queries for more complex import statements.
		for _, q := range ImportQueries {
			queryResults := treeutils.QueryStrings(treeutils.ParseQuery(lang, q), "from", filePath, sourceCode, rootNode)

			Log.Tracef("Query result %q => %v", filePath, queryResults)

			imports = append(imports, queryResults...)
		}

		// Parse errors. Only log them due to many false positives potentially caused by issues
		// such as only parsing a single file at a time so type information from other files is missing.
		if Log.IsLevelEnabled(Log.TraceLevel) {
			treeErrors := treeutils.QueryErrors(lang, sourceCode, rootNode)
			if treeErrors != nil {
				Log.Tracef("TreeSitter query errors: %v", treeErrors)
			}
		}
	}

	result := parser.ParseResult{
		Imports: imports,
		Modules: modules,
	}

	return result, errs
}

// Determine if a node is an import/export statement that may contain a `from` value.
func isImportStatement(node *sitter.Node) bool {
	nodeType := node.Type()

	// Top level `import ... from "..."` statement.
	// Top level `export ... from "..."` statement.
	return nodeType == "import_statement" || nodeType == "export_statement"
}

// Return a Node representing the `from` value of an import/export statement.
func getImportStatementName(importStatement *sitter.Node) *sitter.Node {
	from := importStatement.ChildByFieldName("source")
	if from != nil {
		return from.Child(1)
	}
	return nil
}

// Determine if a node is a module declaration.
func isModuleDeclaration(node *sitter.Node) bool {
	nodeType := node.Type()

	// `declare module "..." [{ ... }]` statement.
	// See: https://www.typescriptlang.org/docs/handbook/modules.html#ambient-modules
	//
	// Example node: (ambient_declaration (module name: (string (string_fragment)) body: (statement_block)))
	return nodeType == "ambient_declaration"
}

// Return a Node representing the module declaration name
func getModuleDeclarationName(node *sitter.Node) *sitter.Node {
	if module := treeutils.GetNodeChildByType(node, "module"); module != nil {
		return treeutils.GetNodeStringField(module, "name")
	}

	return nil
}

// File extension to language key
func filenameToLanguage(filename string) treeutils.Language {
	ext := path.Ext(filename)[1:]
	switch ext {
	case "tsx":
		return treeutils.TypescriptX
	case "jsx":
		return treeutils.TypescriptX
	}

	return treeutils.Typescript
}
