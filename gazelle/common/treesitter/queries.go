package treesitter

import (
	"fmt"
	"strings"
	"sync"

	sitter "github.com/smacker/go-tree-sitter"
)

var ErrorsQuery = `(ERROR) @error`

// A cache of parsed queries per language
var queryCache = make(map[LanguageGrammar]map[string]*sitter.Query)
var queryMutex sync.Mutex

func parseQuery(lang LanguageGrammar, queryStr string) *sitter.Query {
	queryMutex.Lock()
	defer queryMutex.Unlock()

	if queryCache[lang] == nil {
		queryCache[lang] = make(map[string]*sitter.Query)
	}
	if queryCache[lang][queryStr] == nil {
		queryCache[lang][queryStr] = mustNewQuery(lang, queryStr)
	}

	return queryCache[lang][queryStr]
}

// Run a query finding string query matches.
func (tree TreeAst) QueryStrings(query, returnVar string) []string {
	rootNode := tree.SitterTree.RootNode()
	results := make([]string, 0, 5)

	sitterQuery := parseQuery(tree.lang, query)

	// Execute the query.
	qc := sitter.NewQueryCursor()
	qc.Exec(sitterQuery, rootNode)

	// Collect string from the query results.
	for {
		m, ok := qc.NextMatch()
		if !ok {
			break
		}

		result := fetchQueryMatch(sitterQuery, returnVar, m, tree.sourceCode)
		if result != nil {
			resultCode := result.Node.Content(tree.sourceCode)
			results = append(results, resultCode)
		}
	}

	return results
}

// Find and read the `from` QueryCapture from a QueryMatch.
// Filter matches based on captures value using "equals-{name}" vars.
func fetchQueryMatch(query *sitter.Query, name string, m *sitter.QueryMatch, sourceCode []byte) *sitter.QueryCapture {
	var result *sitter.QueryCapture

	for ci, c := range m.Captures {
		cn := query.CaptureNameForId(uint32(ci))

		// Filters where a capture must equal a specific value.
		if strings.HasPrefix(cn, "equals-") {
			if c.Node.Content(sourceCode) != cn[len("equals-"):] {
				return nil
			}
			continue
		}

		if cn == name {
			result = &c
		}
	}

	return result
}

func mustNewQuery(lang LanguageGrammar, queryStr string) *sitter.Query {
	treeQ, err := sitter.NewQuery([]byte(queryStr), toSitterLanguage(lang))
	if err != nil {
		panic(fmt.Sprintf("Failed to create query for %q: %v", queryStr, err))
	}
	return treeQ
}

// Create an error for each parse error.
func (tree TreeAst) QueryErrors() []error {
	node := tree.SitterTree.RootNode()
	if !node.HasError() {
		return nil
	}

	errors := make([]error, 0)

	query := parseQuery(tree.lang, ErrorsQuery)

	// Execute the import query
	qc := sitter.NewQueryCursor()
	qc.Exec(query, node)

	// Collect import statements from the query results
	for {
		m, ok := qc.NextMatch()
		if !ok {
			break
		}

		// Apply predicates to filter results.
		m = qc.FilterPredicates(m, tree.sourceCode)

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

			errors = append(errors, fmt.Errorf(msg+"\n"+arw))
		}
	}

	return errors
}
