package queries

import (
	"log"

	"github.com/aspect-build/aspect-cli/gazelle/host/plugin"
)

func RunQueries(queryType plugin.QueryType, fileName string, sourceCode []byte, queries plugin.NamedQueries, queryResults chan *plugin.QueryProcessorResult) error {
	switch queryType {
	case plugin.QueryTypeAst:
		return runPluginTreeQueries(fileName, sourceCode, queries, queryResults)
	case plugin.QueryTypeRegex:
		return runRegexQueries(sourceCode, queries, queryResults)
	case plugin.QueryTypeJson:
		return runJsonQueries(fileName, sourceCode, queries, queryResults)
	case plugin.QueryTypeYaml:
		return runYamlQueries(fileName, sourceCode, queries, queryResults)
	case plugin.QueryTypeRaw:
		return runRawQueries(fileName, sourceCode, queries, queryResults)
	default:
		log.Panicf("Unknown query type: %v", queryType)
		return nil
	}
}

func runRawQueries(fileName string, sourceCode []byte, queries plugin.NamedQueries, queryResults chan *plugin.QueryProcessorResult) error {
	sourceCodeStr := string(sourceCode)
	for key, _ := range queries {
		queryResults <- &plugin.QueryProcessorResult{
			Key:    key,
			Result: sourceCodeStr,
		}
	}
	return nil
}
