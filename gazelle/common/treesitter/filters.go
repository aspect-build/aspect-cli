package treesitter

import (
	"regexp"

	sitter "github.com/smacker/go-tree-sitter"
)

// An extension of the go-tree-sitter QueryCursor.FilterPredicates() to add additional filtering.
//
// Limited implementation of predicates implemented in go-tree-sitter:
//   - https://github.com/smacker/go-tree-sitter/blob/c5d1f3f5f99edffd6f1e2f53de46996717717dd2/bindings.go#L1081
//
// Examples of additional standard tree-sitter predicates:
//   - https://tree-sitter.github.io/tree-sitter/using-parsers#predicates
//
// Predicates implemented here:
//   - eq?
//   - match?
func matchesAllPredicates(q *sitter.Query, m *sitter.QueryMatch, qc *sitter.QueryCursor, input []byte) bool {
	qm := &sitter.QueryMatch{
		ID:           m.ID,
		PatternIndex: m.PatternIndex,
	}

	predicates := q.PredicatesForPattern(uint32(qm.PatternIndex))
	if len(predicates) == 0 {
		return true
	}

	// check each predicate against the match
	for _, steps := range predicates {
		operator := q.StringValueForId(steps[0].ValueId)

		switch operator {
		case "eq?", "not-eq?":
			isPositive := operator == "eq?"

			expectedCaptureNameLeft := q.CaptureNameForId(steps[1].ValueId)

			if steps[2].Type == sitter.QueryPredicateStepTypeCapture {
				expectedCaptureNameRight := q.CaptureNameForId(steps[2].ValueId)

				var nodeLeft, nodeRight *sitter.Node

				for _, c := range m.Captures {
					captureName := q.CaptureNameForId(c.Index)

					if captureName == expectedCaptureNameLeft {
						nodeLeft = c.Node
					}
					if captureName == expectedCaptureNameRight {
						nodeRight = c.Node
					}

					if nodeLeft != nil && nodeRight != nil {
						if (nodeLeft.Content(input) == nodeRight.Content(input)) != isPositive {
							return false
						}
					}
				}
			} else {
				expectedValueRight := q.StringValueForId(steps[2].ValueId)

				for _, c := range m.Captures {
					captureName := q.CaptureNameForId(c.Index)

					if expectedCaptureNameLeft != captureName {
						continue
					}

					if (c.Node.Content(input) == expectedValueRight) != isPositive {
						return false
					}
				}
			}

		case "match?", "not-match?":
			isPositive := operator == "match?"

			expectedCaptureName := q.CaptureNameForId(steps[1].ValueId)
			regex := regexp.MustCompile(q.StringValueForId(steps[2].ValueId))

			for _, c := range m.Captures {
				captureName := q.CaptureNameForId(c.Index)
				if expectedCaptureName != captureName {
					continue
				}

				if regex.MatchString(c.Node.Content(input)) != isPositive {
					return false
				}
			}
		}
	}

	return true
}
