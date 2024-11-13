package treesitter

import (
	"fmt"
	"strings"
	"sync"

	BazelLog "aspect.build/cli/pkg/logger"

	sitter "github.com/smacker/go-tree-sitter"
)

var ErrorsQuery = `(ERROR) @error`

// A cache of parsed queries per language
var queryCache = sync.Map{}

func parseQuery(lang LanguageGrammar, queryStr string) *sitterQuery {
	key := string(lang) + ":" + queryStr

	q, found := queryCache.Load(key)
	if !found {
		q, _ = queryCache.LoadOrStore(key, mustNewQuery(lang, queryStr))
	}
	return q.(*sitterQuery)
}

// Run a query finding string query matches.
func (tree *treeAst) QueryStrings(query, returnVar string) []string {
	rootNode := tree.sitterTree.RootNode()
	results := make([]string, 0, 5)

	sitterQuery := parseQuery(tree.lang, query)

	// Execute the query.
	qc := sitter.NewQueryCursor()
	qc.Exec(sitterQuery.q, rootNode)

	// Collect string from the query results.
	for {
		m, ok := qc.NextMatch()
		if !ok {
			break
		}

		// Apply predicates to filter results.
		if !matchesAllPredicates(sitterQuery, m, qc, tree.sourceCode) {
			continue
		}

		result := fetchQueryMatch(sitterQuery, returnVar, m, tree.sourceCode)
		if result != nil {
			resultCode := result.Node.Content(tree.sourceCode)
			results = append(results, resultCode)
		}
	}

	return results
}
func (tree *treeAst) RootNode() *sitter.Node {
	return tree.sitterTree.RootNode()
}

type queryResult struct {
	QueryCaptures map[string]string
}

var _ ASTQueryResult = (*queryResult)(nil)

func (qr queryResult) Captures() map[string]string {
	return qr.QueryCaptures
}

func (tree *treeAst) Query(query string) <-chan ASTQueryResult {
	q := parseQuery(tree.lang, query)
	rootNode := tree.sitterTree.RootNode()

	out := make(chan ASTQueryResult)

	// Execute the query.
	go func() {
		qc := sitter.NewQueryCursor()
		qc.Exec(q.q, rootNode)

		for {
			m, ok := qc.NextMatch()
			if !ok {
				break
			}

			// Filter the capture results
			if !matchesAllPredicates(q, m, qc, tree.sourceCode) {
				continue
			}

			out <- queryResult{QueryCaptures: tree.mapQueryMatchCaptures(m, q)}
		}

		close(out)
	}()

	return out
}

func (tree *treeAst) mapQueryMatchCaptures(m *sitter.QueryMatch, q *sitterQuery) map[string]string {
	captures := make(map[string]string, len(m.Captures))
	for _, c := range m.Captures {
		name := q.CaptureNameForId(c.Index)
		if v, e := captures[name]; e {
			panic(fmt.Sprintf("Multiple captures for %q: %q and %q", name, v, c.Node.Content(tree.sourceCode)))
		}

		captures[name] = c.Node.Content(tree.sourceCode)
	}

	return captures
}

// Find and read the `from` QueryCapture from a QueryMatch.
// Filter matches based on captures value using "equals-{name}" vars.
func fetchQueryMatch(query *sitterQuery, name string, m *sitter.QueryMatch, sourceCode []byte) *sitter.QueryCapture {
	var result *sitter.QueryCapture

	for _, c := range m.Captures {
		cn := query.CaptureNameForId(c.Index)

		// Filters where a capture must equal a specific value.
		if strings.HasPrefix(cn, "equals-") {
			if c.Node.Content(sourceCode) != cn[len("equals-"):] {
				return nil
			}
			continue
		}

		if cn == name {
			if result != nil {
				BazelLog.Errorf("Multiple matches for %q", name)
			}
			result = &c
		}
	}

	return result
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

	query := parseQuery(tree.lang, ErrorsQuery)

	// Execute the import query
	qc := sitter.NewQueryCursor()
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
