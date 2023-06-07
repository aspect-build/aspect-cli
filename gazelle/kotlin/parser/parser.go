package parser

import (
	"context"
	"fmt"
	"os"
	"strings"

	treeutils "aspect.build/cli/gazelle/common/treesitter"
	"github.com/emirpasic/gods/sets/treeset"
	sitter "github.com/smacker/go-tree-sitter"
	"github.com/smacker/go-tree-sitter/kotlin"
)

type Parser interface {
	ParseImports(filePath, source string) ([]string, []error)
	ParsePackage(filePath, source string) (string, []error)
}

type treeSitterParser struct {
	Parser

	parser *sitter.Parser
}

func NewParser() Parser {
	sitter := sitter.NewParser()
	sitter.SetLanguage(kotlin.GetLanguage())

	p := treeSitterParser{
		parser: sitter,
	}

	return &p
}

var KotlinTreeSitterName = "kotlin"
var KotlinLang = kotlin.GetLanguage()

func (p *treeSitterParser) ParsePackage(filePath, source string) (string, []error) {
	var pkg string
	errs := make([]error, 0)

	ctx := context.Background()

	sourceCode := []byte(source)

	tree, err := p.parser.ParseCtx(ctx, nil, sourceCode)
	if err != nil {
		errs = append(errs, err)
	}

	if tree != nil {
		rootNode := tree.RootNode()

		// Extract imports from the root nodes
		for i := 0; i < int(rootNode.NamedChildCount()); i++ {
			nodeI := rootNode.NamedChild(i)

			if nodeI.Type() == "package_header" {
				if pkg != "" {
					fmt.Printf("Multiple package declarations found in %s\n", filePath)
					os.Exit(1)
				}

				pkg = readIdentifier(getLoneChild(nodeI, "identifier"), sourceCode)
			}
		}

		treeErrors := treeutils.QueryErrors(KotlinTreeSitterName, KotlinLang, sourceCode, rootNode)
		if treeErrors != nil {
			errs = append(errs, treeErrors...)
		}
	}

	return pkg, errs
}

type KotlinImports struct {
	imports *treeset.Set
}

func (p *treeSitterParser) ParseImports(filePath, source string) ([]string, []error) {
	imports := treeset.NewWithStringComparator()
	errs := make([]error, 0)

	ctx := context.Background()

	sourceCode := []byte(source)

	tree, err := p.parser.ParseCtx(ctx, nil, sourceCode)
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
						for k := 0; k < int(nodeJ.NamedChildCount()); k++ {
							nodeK := nodeJ.NamedChild(k)
							if nodeK.Type() == "identifier" {
								imports.Add(readIdentifier(nodeK, sourceCode))
							}
						}
					}
				}
			}
		}

		treeErrors := treeutils.QueryErrors(KotlinTreeSitterName, KotlinLang, sourceCode, rootNode)
		if treeErrors != nil {
			errs = append(errs, treeErrors...)
		}
	}

	res := make([]string, 0, imports.Size())
	for _, v := range imports.Values() {
		res = append(res, v.(string))
	}
	return res, errs
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

func readIdentifier(node *sitter.Node, sourceCode []byte) string {
	if node.Type() != "identifier" {
		fmt.Printf("Must be type 'identifier': %v - %s", node.Type(), node.Content(sourceCode))
		os.Exit(1)
	}

	var s strings.Builder

	for c := 0; c < int(node.NamedChildCount()); c++ {
		nodeC := node.NamedChild(c)

		// TODO: are there any other node types under an "identifier"

		if nodeC.Type() == "simple_identifier" {
			if s.Len() > 0 {
				s.WriteString(".")
			}
			s.WriteString(nodeC.Content(sourceCode))
		} else if nodeC.Type() != "comment" {
			fmt.Printf("Unexpected node type: %v - %s", nodeC.Type(), nodeC.Content(sourceCode))
			os.Exit(1)
		}
	}

	return s.String()
}
