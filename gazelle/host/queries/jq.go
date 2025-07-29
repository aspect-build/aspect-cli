package queries

import (
	"encoding/json"

	"github.com/itchyny/gojq"
	"golang.org/x/sync/errgroup"

	common "github.com/aspect-build/aspect-cli/gazelle/common"
	"github.com/aspect-build/aspect-cli/gazelle/host/plugin"
)

func runJsonQueries(fileName string, sourceCode []byte, queries plugin.NamedQueries, queryResults chan *plugin.QueryProcessorResult) error {
	var doc interface{}
	err := json.Unmarshal(sourceCode, &doc)
	if err != nil {
		return err
	}

	eg := errgroup.Group{}
	eg.SetLimit(10)

	// TODO: parallelize, see https://github.com/itchyny/gojq/issues/236
	// for issue + potential workaround (patch).
	for key, q := range queries {
		r, err := runJsonQuery(doc, q.Params.(plugin.JsonQueryParams))
		if err != nil {
			return err
		}

		queryResults <- &plugin.QueryProcessorResult{
			Key:    key,
			Result: r,
		}
	}

	return nil
}

func runJsonQuery(doc interface{}, query string) (interface{}, error) {
	q, err := common.ParseJsonQuery(query)
	if err != nil {
		return nil, err
	}

	matches := make([]interface{}, 0)

	iter := q.Run(doc)
	for {
		v, ok := iter.Next()
		if !ok {
			break
		}

		// See error snippet and notes: https://pkg.go.dev/github.com/itchyny/gojq#readme-usage-as-a-library
		if err, ok := v.(error); ok {
			if err, ok := err.(*gojq.HaltError); ok && err.Value() == nil {
				break
			}
			return nil, err
		}

		matches = append(matches, v)
	}

	return matches, nil
}
