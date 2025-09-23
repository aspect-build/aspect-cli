package treesitter

import (
	"fmt"
	"iter"
	"strings"
	"sync"

	BazelLog "github.com/aspect-build/aspect-cli/gazelle/common/logger"

	sitter "github.com/smacker/go-tree-sitter"
)

var ErrorsQuery = `(ERROR) @error`

type TreeQuery interface {
}

// A cache of parsed queries per language
var queryCache = sync.Map{}

func GetQuery(lang LanguageGrammar, queryStr string) *sitterQuery {
	key := string(lang) + ":" + queryStr

	q, found := queryCache.Load(key)
	if !found {
		q, _ = queryCache.LoadOrStore(key, mustNewQuery(lang, queryStr))
	}
	return q.(*sitterQuery)
}

type queryResult struct {
	QueryCaptures map[string]string
}

var _ ASTQueryResult = (*queryResult)(nil)

func (qr queryResult) Captures() map[string]string {
	return qr.QueryCaptures
}

func (tree *treeAst) Query(query TreeQuery) iter.Seq[ASTQueryResult] {
	return func(yield func(ASTQueryResult) bool) {
		q := query.(*sitterQuery)

		// Execute the query.
		qc := sitter.NewQueryCursor()
		defer qc.Close()
		qc.Exec(q.q, tree.sitterTree.RootNode())

		for {
			m, ok := qc.NextMatch()
			if !ok {
				break
			}

			// Filter the capture results
			if !matchesAllPredicates(q, m, qc, tree.sourceCode) {
				continue
			}

			r := &queryResult{QueryCaptures: tree.mapQueryMatchCaptures(m, q)}
			if !yield(r) {
				break
			}
		}
	}
}

func (tree *treeAst) mapQueryMatchCaptures(m *sitter.QueryMatch, q *sitterQuery) map[string]string {
	captures := make(map[string]string, len(m.Captures))
	for _, c := range m.Captures {
		name := q.CaptureNameForId(c.Index)
		captures[name] = c.Node.Content(tree.sourceCode)
	}

	return captures
}

func mustNewTreeQuery(lang LanguageGrammar, query string) *sitter.Query {
	treeQ, err := sitter.NewQuery([]byte(query), toSitterLanguage(lang))
	if err != nil {
		BazelLog.Fatalf("Failed to create query for %q: %v", query, err)
	}
	return treeQ
}

// Create an error for each parse error.
func (tree *treeAst) QueryErrors() []error {
	node := tree.sitterTree.RootNode()
	if !node.HasError() {
		return nil
	}

	errors := make([]error, 0)

	query := GetQuery(tree.lang, ErrorsQuery)

	// Execute the import query
	qc := sitter.NewQueryCursor()
	defer qc.Close()
	qc.Exec(query.q, node)

	// Collect import statements from the query results
	for {
		m, ok := qc.NextMatch()
		if !ok {
			break
		}

		// Apply predicates to filter results.
		if !matchesAllPredicates(query, m, qc, tree.sourceCode) {
			continue
		}

		for _, c := range m.Captures {
			at := c.Node
			atStart := at.StartPoint()
			show := c.Node

			// Navigate up the AST to include the full source line
			if atStart.Column > 0 {
				for show.StartPoint().Row > 0 && show.StartPoint().Row == atStart.Row {
					show = show.Parent()
				}
			}

			// Extract only that line from the parent Node
			lineI := int(atStart.Row - show.StartPoint().Row)
			colI := int(atStart.Column)
			line := strings.Split(show.Content(tree.sourceCode), "\n")[lineI]

			pre := fmt.Sprintf("     %d: ", atStart.Row+1)
			msg := pre + line
			arw := strings.Repeat(" ", len(pre)+colI) + "^"

			errors = append(errors, fmt.Errorf("%s\n%s", msg, arw))
		}
	}

	return errors
}
