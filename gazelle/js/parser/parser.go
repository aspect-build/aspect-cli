package parser

import (
	"path"
	"regexp"
	"strings"

	Log "aspect.build/cli/pkg/logger"
	sitter "github.com/smacker/go-tree-sitter"

	treeutils "aspect.build/cli/gazelle/common/treesitter"
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

// Queries finding import statements, tagging such Nodes as 'from' captures.
// Optionally filtering captures using 'equals-{name}' vars and #eq? statements.
var importQueries = map[string]string{
	// Dynamic `import("...")` statement
	"dynamic_esm_import": `
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
	"require": `
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

var tripleSlashRe = regexp.MustCompile(`^///\s*<reference\s+(?:lib|path|types)\s*=\s*"(?P<lib>[^"]+)"`)

func ParseSource(filePath, sourceCodeStr string) (ParseResult, []error) {
	imports := make([]string, 0, 5)
	modules := make([]string, 0)
	errs := make([]error, 0)

	sourceCode := []byte(sourceCodeStr)
	lang := filenameToLanguage(filePath)

	// Parse the source code
	tree, err := treeutils.ParseSourceCode(lang, filePath, sourceCode)
	if err != nil {
		errs = append(errs, err)
	}

	if tree != nil {
		rootNode := tree.(treeutils.TreeAst).SitterTree.RootNode()

		// Quick pass over root nodes to find top level imports and modules
		for i := 0; i < int(rootNode.NamedChildCount()); i++ {
			node := rootNode.NamedChild(i)

			if isImportStatement(node) {
				if rootImportNode := getImportStatementName(node); rootImportNode != nil {
					rootImportPath := rootImportNode.Content(sourceCode)

					Log.Tracef("ESM import %q: %v", filePath, rootImportPath)

					imports = append(imports, rootImportPath)
				}
			} else if isModuleDeclaration(node) {
				if modDeclNameNode := getModuleDeclarationName(node); modDeclNameNode != nil {
					modDeclName := modDeclNameNode.Content(sourceCode)

					Log.Tracef("Module declaration %q: %v", filePath, modDeclName)

					modules = append(modules, modDeclName)
				}

				// Import/export statements within a module declaration.
				if moduleRootNode := treeutils.GetNodeChildByTypePath(node, "module", "statement_block"); moduleRootNode != nil {
					for j := 0; j < int(moduleRootNode.NamedChildCount()); j++ {
						moduleNode := moduleRootNode.NamedChild(j)

						if isImportStatement(moduleNode) {
							if moduleImportNode := getImportStatementName(moduleNode); moduleImportNode != nil {
								moduleImport := moduleImportNode.Content(sourceCode)

								Log.Tracef("Module declaration import %q: %v", filePath, moduleImport)

								imports = append(imports, moduleImport)
							}
						}
					}
				}
			} else if isPotentialTripleSlashDirective(node, sourceCode) {
				typesImport, isTripleSlash := getTripleSlashDirectiveModule(node, sourceCode)
				if isTripleSlash {
					imports = append(imports, typesImport)
				}
			}
		}

		// Extra queries for more complex import statements.
		for key, q := range importQueries {
			queryResults := tree.QueryStrings(q, "from")

			if len(queryResults) > 0 {
				Log.Tracef("Import Query (%s) result %q => %v", key, filePath, queryResults)

				imports = append(imports, queryResults...)
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

	return result, errs
}

// Determine if a node is potentially a triple-slash directive.
// To be used before running a more expensive check and triple-slash directive extraction.
// See: https://www.typescriptlang.org/docs/handbook/triple-slash-directives.html
func isPotentialTripleSlashDirective(node *sitter.Node, sourceCode []byte) bool {
	nodeType := node.Type()
	if nodeType != "comment" {
		return false
	}

	comment := node.Content(sourceCode)

	return strings.HasPrefix(comment, "///")
}

// Extract a /// <reference> from a comment node
// Note: could also potentially use a treesitter query such as:
// /  `(program (comment) @result (#match? @c "^///\\s*<reference\\s+(lib|types|path)\\s*=\\s*\"[^\"]+\""))`
func getTripleSlashDirectiveModule(node *sitter.Node, sourceCode []byte) (string, bool) {
	comment := node.Content(sourceCode)
	submatches := tripleSlashRe.FindAllStringSubmatchIndex(comment, -1)
	if len(submatches) != 1 {
		return "", false
	}

	lib := tripleSlashRe.ExpandString(make([]byte, 0), "$lib", comment, submatches[0])
	return string(lib), len(lib) > 0
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
