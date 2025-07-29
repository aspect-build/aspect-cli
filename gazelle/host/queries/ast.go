package queries

import (
	treeutils "github.com/aspect-build/aspect-cli/gazelle/common/treesitter"
	"github.com/aspect-build/aspect-cli/gazelle/host/plugin"
	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
)

func runPluginTreeQueries(fileName string, sourceCode []byte, queries plugin.NamedQueries, queryResults chan *plugin.QueryProcessorResult) error {
	ast, err := treeutils.ParseSourceCode(toTreeGrammar(fileName, queries), fileName, sourceCode)
	if err != nil {
		return err
	}
	defer ast.Close()

	// Parse errors. Only log them due to many false positives.
	// TODO: what false positives? See js plugin where this is from
	if BazelLog.IsLevelEnabled(BazelLog.TraceLevel) {
		treeErrors := ast.QueryErrors()
		if treeErrors != nil {
			BazelLog.Tracef("TreeSitter query errors: %v", treeErrors)
		}
	}

	// TODO: look into running queries in parallel on the same AST
	for key, query := range queries {
		params := query.Params.(plugin.AstQueryParams)
		treeQuery := treeutils.GetQuery(treeutils.LanguageGrammar(params.Grammar), params.Query)

		// TODO: delay collection from channel until first read?
		// Then it must be cached for later reads...
		matches := plugin.QueryMatches(nil)
		for r := range ast.Query(treeQuery) {
			matches = append(matches, plugin.NewQueryMatch(r.Captures(), nil))
		}

		queryResults <- &plugin.QueryProcessorResult{
			Key:    key,
			Result: matches,
		}
	}

	return nil
}

func toTreeGrammar(fileName string, queries plugin.NamedQueries) treeutils.LanguageGrammar {
	// TODO: fail if queries on the same file use different languages?

	for _, q := range queries {
		grammar := q.Params.(plugin.AstQueryParams).Grammar
		if grammar != "" {
			return treeutils.LanguageGrammar(grammar)
		}
	}

	return treeutils.PathToLanguage(fileName)
}
