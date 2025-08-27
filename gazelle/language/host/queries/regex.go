package queries

import (
	"regexp"

	common "github.com/aspect-build/aspect-cli/gazelle/common"
	"github.com/aspect-build/aspect-cli/gazelle/language/host/plugin"
	"golang.org/x/sync/errgroup"
)

func runRegexQueries(sourceCode []byte, queries plugin.NamedQueries, queryResults chan *plugin.QueryProcessorResult) error {
	eg := errgroup.Group{}
	eg.SetLimit(10)

	for key, q := range queries {
		eg.Go(func() error {
			queryResults <- &plugin.QueryProcessorResult{
				Key:    key,
				Result: runRegexQuery(sourceCode, common.ParseRegex(q.Params.(plugin.RegexQueryParams))),
			}
			return nil
		})
	}

	return eg.Wait()
}

func runRegexQuery(sourceCode []byte, re *regexp.Regexp) plugin.QueryMatches {
	reMatches := re.FindAllSubmatch(sourceCode, -1)
	if reMatches == nil {
		return nil
	}

	matches := plugin.QueryMatches(nil)

	for _, reMatch := range reMatches {
		captures := make(plugin.QueryCapture)
		for i, name := range re.SubexpNames() {
			if i > 0 && i <= len(reMatch) {
				captures[name] = string(reMatch[i])
			}
		}

		matches = append(matches, plugin.NewQueryMatch(captures, reMatch[0]))
	}

	return matches
}
