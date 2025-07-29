package starzelle

/**
 * Starlark utility libraries for starzelle plugins.
 *
 * See cli/core/gazelle/common/starlark/stdlib for standard non-starzelle starlark libraries.
 */

import (
	"fmt"
	"log"
	"reflect"

	starUtils "github.com/aspect-build/aspect-cli/gazelle/common/starlark/utils"
	"github.com/aspect-build/aspect-cli/gazelle/host/plugin"
	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
	"github.com/bmatcuk/doublestar/v4"
	"go.starlark.net/starlark"
)

func registerConfigureExtension(t *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var pluginId starlark.String
	var properties *starlark.Dict
	var prepare, analyze, declare *starlark.Function

	err := starlark.UnpackArgs(
		"register_configure_extension",
		args,
		kwargs,
		"id", &pluginId,
		"properties?", &properties,
		"prepare?", &prepare,
		"analyze?", &analyze,
		"declare?", &declare,
	)
	if err != nil {
		return nil, err
	}

	t.Local(proxyStateKey).(*starzelleState).addPlugin(
		t,
		pluginId,
		properties,
		prepare,
		analyze,
		declare,
	)

	return starlark.None, nil
}

func registerRuleKind(t *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var kind starlark.String
	var attributes *starlark.Dict

	err := starlark.UnpackArgs(
		"register_rule_kind",
		args,
		kwargs,
		"name", &kind,
		"attributes?", &attributes,
	)
	if err != nil {
		return nil, err
	}

	t.Local(proxyStateKey).(*starzelleState).addKind(t, kind, attributes)
	return starlark.None, nil
}

func readQueryFilters(v starlark.Value) []string {
	if v == nil {
		return nil
	}

	if filterString, ok := v.(starlark.String); ok {
		return []string{readQueryFilter(filterString)}
	}

	return starUtils.ReadList(v, readQueryFilter)
}

func readQueryFilter(v starlark.Value) string {
	return readGlobPatternFatal(v, "query filter")
}

func readGlobPatternFatal(v starlark.Value, what string) string {
	s := v.(starlark.String).GoString()

	if !doublestar.ValidatePattern(s) {
		log.Fatalf("Invalid %s: %v", what, s)
	}

	return s
}

func newAstQuery(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var query starlark.String
	var filterValue starlark.Value
	var grammarValue starlark.String

	err := starlark.UnpackArgs(
		"AstQuery",
		args,
		kwargs,
		"query", &query,
		"grammar?", &grammarValue,
		"filter??", &filterValue,
	)
	if err != nil {
		return nil, err
	}

	return plugin.QueryDefinition{
		Filter:    readQueryFilters(filterValue),
		QueryType: plugin.QueryTypeAst,
		Params: plugin.AstQueryParams{
			Grammar: grammarValue.GoString(),
			Query:   query.GoString(),
		},
	}, nil
}

func newRegexQuery(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var expression starlark.String
	var filterValue starlark.Value

	err := starlark.UnpackArgs(
		"RegexQuery",
		args,
		kwargs,
		"expression", &expression,
		"filter??", &filterValue,
	)
	if err != nil {
		return nil, err
	}

	return plugin.QueryDefinition{
		Filter:    readQueryFilters(filterValue),
		QueryType: plugin.QueryTypeRegex,
		Params:    expression.GoString(),
	}, nil
}

func newRawQuery(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var filterValue starlark.Value

	err := starlark.UnpackArgs(
		"RawQuery",
		args,
		kwargs,
		"filter??", &filterValue,
	)
	if err != nil {
		return nil, err
	}

	return plugin.QueryDefinition{
		Filter:    readQueryFilters(filterValue),
		QueryType: plugin.QueryTypeRaw,
	}, nil
}

func newJsonQuery(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var queryValue starlark.String
	var filterValue starlark.Value

	err := starlark.UnpackArgs(
		"JsonQuery",
		args,
		kwargs,
		"query?", &queryValue,
		"filter??", &filterValue,
	)
	if err != nil {
		return nil, err
	}

	return plugin.QueryDefinition{
		Filter:    readQueryFilters(filterValue),
		QueryType: plugin.QueryTypeJson,
		Params:    queryValue.GoString(),
	}, nil
}

func newYamlQuery(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var queryValue starlark.String
	var filterValue starlark.Value

	err := starlark.UnpackArgs(
		"YamlQuery",
		args,
		kwargs,
		"query?", &queryValue,
		"filter??", &filterValue,
	)
	if err != nil {
		return nil, err
	}

	return plugin.QueryDefinition{
		Filter:    readQueryFilters(filterValue),
		QueryType: plugin.QueryTypeYaml,
		Params:    queryValue.GoString(),
	}, nil
}

func newSourceExtensions(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	return plugin.SourceExtensionsFilter{
		Extensions: starUtils.ReadStringTuple(args),
	}, nil
}

func newSourceGlobs(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	return plugin.SourceGlobFilter{
		Globs: starUtils.ReadTuple(args, readSourceGlob),
	}, nil
}

func readSourceGlob(v starlark.Value) string {
	return readGlobPatternFatal(v, "source glob")
}

func newSourceFiles(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	return plugin.SourceFileFilter{
		Files: starUtils.ReadStringTuple(args),
	}, nil
}

func newPrepareResult(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var queriesValue *starlark.Dict
	var sourcesValue starlark.Value

	err := starlark.UnpackArgs(
		"PrepareResult",
		args,
		kwargs,
		"sources", &sourcesValue,
		"queries??", &queriesValue,
	)
	if err != nil {
		return nil, err
	}

	queries := make(plugin.NamedQueries)
	if queriesValue != nil {
		iter := queriesValue.Iterate()
		defer iter.Done()

		var k starlark.Value
		for iter.Next(&k) {
			v, _, _ := queriesValue.Get(k)

			qd, isQd := v.(plugin.QueryDefinition)
			if !isQd {
				BazelLog.Fatalf("'queries' %v (%s) is not a QueryDefinition", v, reflect.TypeOf(v))
			}

			queries[k.(starlark.String).GoString()] = qd
		}
	}

	var sources map[string][]plugin.SourceFilter
	if sourcesValue != nil {
		// Allow source values as a flat list or a map of lists
		if sourceDict, isDict := (sourcesValue).(*starlark.Dict); isDict {
			sources = starUtils.ReadMap(sourceDict, readSourceFilterEntry)
		} else {
			sources = map[string][]plugin.SourceFilter{
				plugin.DeclareTargetsContextDefaultGroup: readSourceFilterEntry(plugin.DeclareTargetsContextDefaultGroup, sourcesValue),
			}
		}
	}

	return plugin.PrepareResult{
		Sources: sources,
		Queries: queries,
	}, nil
}

func readSourceFilterEntry(k string, v starlark.Value) []plugin.SourceFilter {
	if list, isList := v.(*starlark.List); isList {
		return starUtils.ReadList(list, readSourceFilter)
	} else {
		return []plugin.SourceFilter{readSourceFilter(v)}
	}
}

func readSourceFilter(v starlark.Value) plugin.SourceFilter {
	f, isF := v.(plugin.SourceFilter)

	if !isF {
		BazelLog.Fatalf("'sources' %v is not a SourceFilter", f)
	}

	return f
}

func newImport(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var id, provider, from starlark.String
	var optional starlark.Bool

	err := starlark.UnpackArgs(
		"Import",
		args,
		kwargs,
		"id", &id,
		"provider", &provider,
		"src?", &from,
		"optional?", &optional,
	)
	if err != nil {
		return nil, err
	}

	if id.GoString() == "" || provider.GoString() == "" {
		msg := "Import id and provider cannot be empty\n"
		fmt.Print(msg)
		BazelLog.Fatal(msg)
	}

	return plugin.TargetImport{
		Symbol: plugin.Symbol{
			Id:       id.GoString(),
			Provider: provider.GoString(),
		},
		Optional: bool(optional.Truth()),
		From:     from.GoString(),
	}, nil
}

func newSymbol(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var id, provider starlark.String

	err := starlark.UnpackArgs(
		"Symbol",
		args,
		kwargs,
		"id", &id,
		"provider", &provider,
	)
	if err != nil {
		return nil, err
	}

	return plugin.Symbol{
		Id:       id.GoString(),
		Provider: provider.GoString(),
	}, nil
}

func newLabel(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var repo, pkg, name starlark.String

	err := starlark.UnpackArgs(
		"Label",
		args,
		kwargs,
		"repo?", &repo,
		"pkg?", &pkg,
		"name", &name,
	)
	if err != nil {
		return nil, err
	}

	return plugin.Label{
		Repo: repo.GoString(),
		Pkg:  pkg.GoString(),
		Name: name.GoString(),
	}, nil
}

func newProperty(_ *starlark.Thread, b *starlark.Builtin, args starlark.Tuple, kwargs []starlark.Tuple) (starlark.Value, error) {
	var propType starlark.String
	var propDefault starlark.Value = starlark.None

	err := starlark.UnpackArgs(
		"Property",
		args,
		kwargs,
		"type", &propType,
		"default?", &propDefault,
	)
	if err != nil {
		return nil, err
	}

	return plugin.Property{
		PropertyType: propType.GoString(),
		Default:      starUtils.Read(propDefault),
	}, nil
}

var aspectModule = starUtils.CreateModule(
	"aspect",
	map[string]starUtils.ModuleFunction{
		"register_configure_extension": registerConfigureExtension,
		"register_rule_kind":           registerRuleKind,
		"AstQuery":                     newAstQuery,
		"RegexQuery":                   newRegexQuery,
		"RawQuery":                     newRawQuery,
		"JsonQuery":                    newJsonQuery,
		"YamlQuery":                    newYamlQuery,
		"PrepareResult":                newPrepareResult,
		"Import":                       newImport,
		"Symbol":                       newSymbol,
		"Label":                        newLabel,
		"Property":                     newProperty,
		"SourceExtensions":             newSourceExtensions,
		"SourceGlobs":                  newSourceGlobs,
		"SourceFiles":                  newSourceFiles,
	},
	map[string]starlark.Value{},
)
