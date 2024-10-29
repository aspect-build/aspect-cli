package treesitter

import sitter "github.com/smacker/go-tree-sitter"

// Basic wrapper around sitter.Query to cache tree-sitter cgo calls.
type sitterQuery struct {
	q *sitter.Query

	// Pre-computed and cached query data
	stringValues      []string
	captureNames      []string
	predicatePatterns [][][]sitter.QueryPredicateStep
}

func mustNewQuery(lang LanguageGrammar, query string) *sitterQuery {
	q := mustNewTreeQuery(lang, query)

	captureNames := make([]string, q.CaptureCount())
	for i := uint32(0); i < q.CaptureCount(); i++ {
		captureNames[i] = q.CaptureNameForId(i)
	}

	stringValues := make([]string, q.StringCount())
	for i := uint32(0); i < q.StringCount(); i++ {
		stringValues[i] = q.StringValueForId(i)
	}

	predicatePatterns := make([][][]sitter.QueryPredicateStep, q.PatternCount())
	for i := uint32(0); i < q.PatternCount(); i++ {
		predicatePatterns[i] = q.PredicatesForPattern(i)
	}

	return &sitterQuery{
		q:                 q,
		stringValues:      stringValues,
		captureNames:      captureNames,
		predicatePatterns: predicatePatterns,
	}
}

// Cached query data accessors mirroring the tree-sitter Query signatures.

func (q *sitterQuery) StringValueForId(id uint32) string {
	return q.stringValues[id]
}

func (q *sitterQuery) CaptureNameForId(id uint32) string {
	return q.captureNames[id]
}

func (q *sitterQuery) PredicatesForPattern(patternIndex uint32) [][]sitter.QueryPredicateStep {
	return q.predicatePatterns[patternIndex]
}
