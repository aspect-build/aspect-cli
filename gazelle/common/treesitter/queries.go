package treesitter

import (
	"fmt"
	"strings"
	"sync"

	sitter "github.com/smacker/go-tree-sitter"
)

var ErrorsQuery = `(ERROR) @error`

// A cache of parsed queries per language
var queryCache = make(map[string]map[string]*sitter.Query)
var queryMutex sync.Mutex

func ParseQuery(langName string, lang *sitter.Language, queryStr string) *sitter.Query {
	queryMutex.Lock()
	defer queryMutex.Unlock()

	// TODO: extract langName from sitter.Language?

	if queryCache[queryStr] == nil {
		queryCache[queryStr] = make(map[string]*sitter.Query)
	}
	if queryCache[queryStr][langName] == nil {
		queryCache[queryStr][langName] = mustNewQuery(lang, queryStr)
	}

	return queryCache[queryStr][langName]
}

func mustNewQuery(lang *sitter.Language, queryStr string) *sitter.Query {
	q, err := sitter.NewQuery([]byte(queryStr), lang)
	if err != nil {
		panic(err)
	}
	return q
}

// Create an error for each parse error.
func QueryErrors(langName string, lang *sitter.Language, sourceCode []byte, node *sitter.Node) []error {
	if !node.HasError() {
		return nil
	}

	errors := make([]error, 0)

	query := ParseQuery(langName, lang, ErrorsQuery)

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
