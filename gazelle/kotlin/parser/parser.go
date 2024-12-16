package parser

import (
	"fmt"
	"os"
	"strings"

	treeutils "github.com/aspect-build/aspect-cli/gazelle/common/treesitter"
	"github.com/emirpasic/gods/sets/treeset"
	sitter "github.com/smacker/go-tree-sitter"
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
		rootNode := tree.RootNode()

		// Extract imports from the root nodes
		for i := 0; i < int(rootNode.NamedChildCount()); i++ {
			nodeI := rootNode.NamedChild(i)

			if nodeI.Type() == "import_list" {
				for j := 0; j < int(nodeI.NamedChildCount()); j++ {
					nodeJ := nodeI.NamedChild(j)
					if nodeJ.Type() == "import_header" {
						for k := 0; k < int(nodeJ.ChildCount()); k++ {
							nodeK := nodeJ.Child(k)
							if nodeK.Type() == "identifier" {
								isStar := false
								for l := int(nodeJ.ChildCount()) - 1; l > k; l-- {
									if nodeJ.Child(l).Type() == "wildcard_import" {
										isStar = true
										break
									}
								}

								result.Imports = append(result.Imports, readIdentifier(nodeK, sourceCode, !isStar))

								// Any remaining nodes within nodeJ are comments or wildcards
								break
							}
						}
					}
				}
			} else if nodeI.Type() == "package_header" {
				if result.Package != "" {
					fmt.Printf("Multiple package declarations found in %s\n", filePath)
					os.Exit(1)
				}

				result.Package = readIdentifier(getLoneChild(nodeI, "identifier"), sourceCode, false)
			} else if nodeI.Type() == "function_declaration" {
				nodeJ := getLoneChild(nodeI, "simple_identifier")
				if nodeJ.Content(sourceCode) == "main" {
					result.HasMain = true
				}
			}
		}

		treeErrors := tree.QueryErrors()
		if treeErrors != nil {
			errs = append(errs, treeErrors...)
		}
	}

	return result, errs
}

type KotlinImports struct {
	imports *treeset.Set
}

func getLoneChild(node *sitter.Node, name string) *sitter.Node {
	for i := 0; i < int(node.NamedChildCount()); i++ {
		if node.NamedChild(i).Type() == name {
			return node.NamedChild(i)
		}
	}

	fmt.Printf("Node %v must contain node of type %q", node, name)
	os.Exit(1)
	return nil
}

func readIdentifier(node *sitter.Node, sourceCode []byte, ignoreLast bool) string {
	if node.Type() != "identifier" {
		fmt.Printf("Must be type 'identifier': %v - %s", node.Type(), node.Content(sourceCode))
		os.Exit(1)
	}

	var s strings.Builder

	total := int(node.NamedChildCount())
	if ignoreLast {
		total = total - 1
	}

	for c := 0; c < total; c++ {
		nodeC := node.NamedChild(c)

		// TODO: are there any other node types under an "identifier"

		if nodeC.Type() == "simple_identifier" {
			if s.Len() > 0 {
				s.WriteString(".")
			}
			s.WriteString(nodeC.Content(sourceCode))
		} else if nodeC.Type() != "comment" {
			fmt.Printf("Unexpected node type '%v' within: %s", nodeC.Type(), node.Content(sourceCode))
			os.Exit(1)
		}
	}

	return s.String()
}
