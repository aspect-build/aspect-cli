package treesitter

import sitter "github.com/smacker/go-tree-sitter"

func GetNodeChildByTypePath(node *sitter.Node, childPath ...string) *sitter.Node {
	for _, childType := range childPath {
		node = GetNodeChildByType(node, childType)
		if node == nil {
			return nil
		}
	}

	return node
}

func GetNodeChildByType(node *sitter.Node, childType string) *sitter.Node {
	// TODO: assert only one child of type?

	for i := 0; i < int(node.NamedChildCount()); i++ {
		child := node.NamedChild(i)
		if child.Type() == childType {
			return child
		}
	}

	return nil
}

func GetNodeStringField(node *sitter.Node, name string) *sitter.Node {
	// TODO: assert its a string field, assert only one child
	return node.ChildByFieldName(name).NamedChild(0)
}
