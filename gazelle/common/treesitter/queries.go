package treesitter

import (
	"fmt"
	"strings"
	"sync"

	Log "aspect.build/cli/pkg/logger"
	sitter "github.com/smacker/go-tree-sitter"
)

var ErrorsQuery = `(ERROR) @error`

// A cache of parsed queries per language
var queryCache = make(map[string]map[Language]*sitter.Query)
var queryMutex sync.Mutex

func ParseQuery(lang Language, queryStr string) *sitter.Query {
	queryMutex.Lock()
	defer queryMutex.Unlock()

	// TODO: extract langName from sitter.Language?

	if queryCache[queryStr] == nil {
		queryCache[queryStr] = make(map[Language]*sitter.Query)
	}
	if queryCache[queryStr][lang] == nil {
		queryCache[queryStr][lang] = mustNewQuery(toSitterLanguage(lang), queryStr)
	}

	return queryCache[queryStr][lang]
}

// Run a query finding import query matches.
func QueryStrings(query *sitter.Query, name string, sourcePath string, sourceCode []byte, rootNode *sitter.Node) []string {
	results := make([]string, 0, 5)

	// Execute the import query.
	qc := sitter.NewQueryCursor()
	defer qc.Close()
	qc.Exec(query, rootNode)

	// Collect string from the query results.
	for {
		m, ok := qc.NextMatch()
		if !ok {
			break
		}

		result := fetchFirstQueryMatch(query, name, m, sourceCode)
		if result != nil {
			resultCode := result.Node.Content(sourceCode)
			results = append(results, resultCode)
		}
	}

	return results
}

// Find and read the `from` QueryCapture from a QueryMatch.
// Filter matches based on captures value using "equals-{name}" vars.
func fetchFirstQueryMatch(query *sitter.Query, name string, m *sitter.QueryMatch, sourceCode []byte) *sitter.QueryCapture {
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
			return &c
		}
	}

	// Should never happen. All queries should have a `name` capture.
	Log.Fatalf("No result %q found in query %q", name, query)
	return nil
}

func mustNewQuery(lang *sitter.Language, queryStr string) *sitter.Query {
	q, err := sitter.NewQuery([]byte(queryStr), lang)
	if err != nil {
		panic(err)
	}
	return q
}

// Create an error for each parse error.
func QueryErrors(lang Language, sourceCode []byte, node *sitter.Node) []error {
	if !node.HasError() {
		return nil
	}

	errors := make([]error, 0)

	query := ParseQuery(lang, ErrorsQuery)

	// Execute the import query
	qc := sitter.NewQueryCursor()
	defer qc.Close()
	qc.Exec(query, node)

	// Collect import statements from the query results
	for {
		m, ok := qc.NextMatch()
		if !ok {
			break
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
			line := strings.Split(show.Content(sourceCode), "\n")[lineI]

			pre := fmt.Sprintf("     %d: ", atStart.Row+1)
			msg := pre + line
			arw := strings.Repeat(" ", len(pre)+colI) + "^"

			errors = append(errors, fmt.Errorf(msg+"\n"+arw))
		}
	}

	return errors
}
