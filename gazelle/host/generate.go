package gazelle

import (
	"crypto"
	"encoding/gob"
	"encoding/hex"
	"fmt"
	"maps"
	"os"
	"path"
	"slices"
	"sort"
	"strings"
	"sync"

	common "github.com/aspect-build/aspect-cli/gazelle/common"
	"github.com/aspect-build/aspect-cli/gazelle/common/cache"
	"github.com/aspect-build/aspect-cli/gazelle/host/plugin"
	queryRunner "github.com/aspect-build/aspect-cli/gazelle/host/queries"
	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
	gazelleLabel "github.com/bazelbuild/bazel-gazelle/label"
	gazelleLanguage "github.com/bazelbuild/bazel-gazelle/language"
	gazelleRule "github.com/bazelbuild/bazel-gazelle/rule"
	"golang.org/x/sync/errgroup"
)

const (
	// TODO: move to common
	MaxWorkerCount = 12
)

const (
	targetAttrValues     = "__target_attr_values"
	targetDeclarationKey = "__target_declaration"
	targetPluginKey      = "__target_plugin"
)

// Gazelle GenerateRules phase - declare:
//   - which rules to delete (GenerateResult.Empty)
//   - which rules to create (or merge with existing) and their associated metadata (GenerateResult.Gen + GenerateResult.Imports)
func (host *GazelleHost) GenerateRules(args gazelleLanguage.GenerateArgs) gazelleLanguage.GenerateResult {
	BazelLog.Tracef("GenerateRules(%s): %s", GazelleLanguageName, args.Rel)

	cfg := args.Config.Exts[GazelleLanguageName].(*BUILDConfig)

	// Mark this BUILDConfig as generated since it is having real rules generated.
	cfg.generated = true

	queryCache := cache.Get(args.Config)

	// Stage 1:
	// Collect source files indexed for multiple purposes such as:
	//  - iterating over all source files per plugin
	//  - iterating over plugins per source file
	//  - iterating over source files by plugin file group
	pluginSourceFiles, sourceFilePlugins, pluginSourceGroupFiles := host.collectSourceFilesByPlugin(cfg, args)

	// Run queries on source files and collect results
	eg := errgroup.Group{}
	eg.SetLimit(100)

	// Stage 2:
	// Parse and query source files and collect results

	sourceFileQueryResults := make(map[string]plugin.QueryResults, len(sourceFilePlugins))
	sourceFileQueryResultsLock := sync.Mutex{}

	// Parse and query source files
	for sourceFile, pluginIds := range sourceFilePlugins {
		// Collect all queries for this source file from all plugins
		queries := make(plugin.NamedQueries)
		for _, pluginId := range pluginIds {
			prep := cfg.pluginPrepareResults[pluginId]
			for queryId, query := range prep.GetQueriesForFile(sourceFile) {
				queries[fmt.Sprintf("%s|%s", pluginId, queryId)] = query
			}
		}

		if len(queries) == 0 {
			continue
		}

		eg.Go(func() error {
			p := path.Join(args.Rel, sourceFile)
			queryResults, err := host.runSourceQueries(queryCache, queries, args.Config.RepoRoot, p)
			if err != nil {
				msg := fmt.Sprintf("Querying source file %q: %v", p, err)
				fmt.Printf("%s\n", msg)
				BazelLog.Error(msg)
				return nil
			}

			sourceFileQueryResultsLock.Lock()
			defer sourceFileQueryResultsLock.Unlock()
			sourceFileQueryResults[sourceFile] = queryResults
			return nil
		})
	}

	if err := eg.Wait(); err != nil {
		BazelLog.Errorf("Collect plugin sources error: %v", err)
	}

	// Build the TargetSource for each file for each plugin.
	pluginTargetSources := make(map[plugin.PluginId]map[string]plugin.TargetSource, len(cfg.pluginPrepareResults))
	for pluginId, _ := range cfg.pluginPrepareResults {
		pluginSrcs := pluginSourceFiles[pluginId]

		queryPrefix := fmt.Sprintf("%s|", pluginId)

		// Collect the query results for this plugin's source files
		targetSources := make(map[string]plugin.TargetSource, len(pluginSrcs))
		for _, f := range pluginSrcs {
			queryResults := make(plugin.QueryResults)
			for queryId, results := range sourceFileQueryResults[f] {
				if strings.HasPrefix(queryId, queryPrefix) {
					queryResults[queryId[len(queryPrefix):]] = results
				}
			}

			targetSources[f] = plugin.TargetSource{
				Path:         f,
				QueryResults: queryResults,
			}
		}

		pluginTargetSources[pluginId] = targetSources
	}

	// Stage 3:
	// Analyze each plugin source file.
	for pluginId, prep := range cfg.pluginPrepareResults {
		if targetSources := pluginTargetSources[pluginId]; len(targetSources) > 0 {
			eg.Go(func() error {
				host.analyzePluginTargetSources(pluginId, prep, targetSources)
				return nil
			})
		}
	}

	if err := eg.Wait(); err != nil {
		BazelLog.Errorf("Plugin source analysis error: %v", err)
	}

	// Stage 4:
	// Generate target actions for each plugin
	pluginTargetActions := make(map[plugin.PluginId][]plugin.TargetAction, len(cfg.pluginPrepareResults))
	pluginTargetsLock := sync.Mutex{}
	for pluginId, prep := range cfg.pluginPrepareResults {
		eg.Go(func() error {
			// Group the TargetSource's into the source groups for the plugin.
			pluginTargetGroups := plugin.TargetSources{}

			for groupId, _ := range prep.Sources {
				files := pluginSourceGroupFiles[pluginId][groupId]

				// Add the TargetSource for each file in the group, even if empty.
				pluginTargetGroups[groupId] = make([]plugin.TargetSource, 0, len(files))
				for _, f := range files {
					pluginTargetGroups[groupId] = append(pluginTargetGroups[groupId], pluginTargetSources[pluginId][f])
				}
			}

			// If no default group exists create one with all sources.
			if _, hasDefaultGroup := pluginTargetGroups[plugin.DeclareTargetsContextDefaultGroup]; !hasDefaultGroup {
				pluginTargetGroups[plugin.DeclareTargetsContextDefaultGroup] = slices.Collect(maps.Values(pluginTargetSources[pluginId]))
			}

			// Use the collected sources and analysis to generate rules
			actions := host.generateTargets(pluginId, prep, pluginTargetGroups)

			// Lock for the assignment into the cross-thread pluginTargets
			pluginTargetsLock.Lock()
			defer pluginTargetsLock.Unlock()
			pluginTargetActions[pluginId] = actions

			return nil
		})
	}

	if err := eg.Wait(); err != nil {
		BazelLog.Errorf("Unknown GenerateRules(%s) error: %v", GazelleLanguageName, err)
	}

	// Stage 5:
	// Apply plugin actions
	return host.convertPlugActionsToGenerateResult(pluginTargetActions, args)
}

func applyRemoveAction(args gazelleLanguage.GenerateArgs, result *gazelleLanguage.GenerateResult, rm plugin.RemoveTargetAction) *gazelleRule.Rule {
	if args.File == nil {
		return nil
	}

	for _, r := range args.File.Rules {
		if r.Name() == rm.Name {
			kind := rm.Kind
			if rm.Kind == "" {
				kind = r.Kind() // TODO: need to reverse map_kind?
			}
			result.Empty = append(result.Empty, gazelleRule.NewRule(kind, r.Name()))
			return r
		}
	}
	return nil
}

func (host *GazelleHost) convertPlugActionsToGenerateResult(pluginActions map[string][]plugin.TargetAction, args gazelleLanguage.GenerateArgs) gazelleLanguage.GenerateResult {
	var result gazelleLanguage.GenerateResult

	// Iterate over the pluginIds[] in a deterministic order
	// instead of iterating over the plugins[] or pluginActions[pluginId] map
	for _, pluginId := range host.pluginIds {
		for _, action := range pluginActions[pluginId] {
			host.applyPluginAction(args, pluginId, action, &result)
		}
	}

	return result
}

func (host *GazelleHost) applyPluginAction(args gazelleLanguage.GenerateArgs, pluginId plugin.PluginId, action plugin.TargetAction, result *gazelleLanguage.GenerateResult) {
	switch action.(type) {
	case plugin.RemoveTargetAction:
		// If marked for removal simply add to the empty list and continue
		if removed := applyRemoveAction(args, result, action.(plugin.RemoveTargetAction)); removed != nil {
			BazelLog.Debugf("GenerateRules remove target: %s %s(%q)", args.Rel, removed.Kind(), removed.Name())
		}
	case plugin.AddTargetAction:
		// Check for name-collisions with the rule being generated.
		target := action.(plugin.AddTargetAction).TargetDeclaration
		colError := common.CheckCollisionErrors(target.Name, target.Kind, host.sourceRuleKinds, args)
		if colError != nil {
			fmt.Fprintf(os.Stderr, "Source rule generation error: %v\n", colError)
			os.Exit(1)
		}

		// Generate the gazelle Rule to be added/merged into the BUILD file.
		rule := convertPluginTargetDeclaration(args, pluginId, target)

		result.Gen = append(result.Gen, rule)
		result.Imports = append(result.Imports, rule.PrivateAttr(targetAttrValues))

		BazelLog.Tracef("GenerateRules(%s) add target: %s %s(%q)", GazelleLanguageName, args.Rel, target.Kind, target.Name)
	default:
		BazelLog.Fatalf("Unknown plugin action type: %T", action)
	}
}

type attributeValue struct {
	singleton bool
	values    []interface{}
	imports   []plugin.TargetImport
}

func convertPluginTargetDeclaration(args gazelleLanguage.GenerateArgs, pluginId plugin.PluginId, target plugin.TargetDeclaration) *gazelleRule.Rule {
	targetRule := gazelleRule.NewRule(target.Kind, target.Name)

	ruleAttrs := make(map[string]*attributeValue, len(target.Attrs))

	targetRule.SetPrivateAttr(targetPluginKey, pluginId)
	targetRule.SetPrivateAttr(targetDeclarationKey, target)
	targetRule.SetPrivateAttr(targetAttrValues, ruleAttrs)

	for attr, val := range target.Attrs {
		attrValue, attrImports, isArray := convertPluginAttribute(args, val)

		// TODO: verify 'attr' is resolveable if len(attrImports) > 0
		ruleAttrs[attr] = &attributeValue{
			singleton: !isArray,
			imports:   attrImports,
			values:    attrValue,
		}

		// Update the attribute if any non-import was specified
		if len(attrValue) > 0 {
			if isArray {
				// An array of values taken as-is
				targetRule.SetAttr(attr, attrValue)
			} else if attrValue[0] == nil {
				// A single nil value is the same as deleting
				targetRule.DelAttr(attr)
			} else {
				// Otherwise use the single value
				targetRule.SetAttr(attr, attrValue[0])
			}
		}
	}

	return targetRule
}

func convertPluginAttribute(args gazelleLanguage.GenerateArgs, val interface{}) ([]interface{}, []plugin.TargetImport, bool) {
	if a, isArray := val.([]interface{}); isArray {
		var r []interface{}
		var i []plugin.TargetImport
		for _, v := range a {
			newR, newI, _ := convertPluginAttribute(args, v)
			if newR != nil {
				r = append(r, newR...)
			}
			if newI != nil {
				i = append(i, newI...)
			}
		}
		return r, i, true
	}

	if targetImport, isImport := val.(plugin.TargetImport); isImport {
		return nil, []plugin.TargetImport{targetImport}, false
	}

	// Convert plugin.Label to a gazelle Label
	if l, isLabel := val.(plugin.Label); isLabel {
		val = gazelleLabel.New(l.Repo, l.Pkg, l.Name)
	}

	// Normalize gazelle labels to be relative to the BUILD file
	if l, isLabel := val.(gazelleLabel.Label); isLabel {
		// TODO: also convert the `args.Config.RepoName` repo to relative?
		return []interface{}{l.Rel("", args.Rel)}, nil, false
	}

	return []interface{}{val}, nil, false
}

func init() {
	// Ensure types used in cache key computation are known to the gob encoder
	gob.Register(plugin.NamedQueries{})
	gob.Register(plugin.QueryDefinition{})
	gob.Register(plugin.QueryType(""))
	gob.Register(plugin.AstQueryParams{})
	gob.Register(plugin.RegexQueryParams(""))
	gob.Register(plugin.JsonQueryParams(""))
}

func computeQueriesCacheKey(queries plugin.NamedQueries) string {
	cacheDigest := crypto.MD5.New()

	keys := make([]string, 0, len(queries))
	for key := range queries {
		keys = append(keys, key)
	}
	sort.Strings(keys)

	e := gob.NewEncoder(cacheDigest)
	for _, key := range keys {
		if err := e.Encode(key); err != nil {
			BazelLog.Fatalf("Failed to encode query key %q: %v", key, err)
		}
		if err := e.Encode(queries[key]); err != nil {
			BazelLog.Fatalf("Failed to encode query value %q: %v", queries[key], err)
		}
	}

	return hex.EncodeToString(cacheDigest.Sum(nil))
}

func (host *GazelleHost) runSourceQueries(queryCache cache.Cache, queries plugin.NamedQueries, baseDir, f string) (plugin.QueryResults, error) {
	queriesHash := computeQueriesCacheKey(queries)

	var qr plugin.QueryResults

	r, _, err := queryCache.LoadOrStoreFile(baseDir, f, queriesHash, func(p string, sourceCode []byte) (any, error) {
		return host.runSourceCodeQueries(queries, sourceCode, f)
	})

	if r != nil {
		qr = r.(plugin.QueryResults)
	}

	return qr, err
}

func (host *GazelleHost) runSourceCodeQueries(queries plugin.NamedQueries, sourceCode []byte, f string) (plugin.QueryResults, error) {
	// Split queries by type to invoke in batches
	queriesByType := make(map[plugin.QueryType]plugin.NamedQueries)
	for key, query := range queries {
		if queriesByType[query.QueryType] == nil {
			queriesByType[query.QueryType] = make(plugin.NamedQueries)
		}
		queriesByType[query.QueryType][key] = query
	}

	queryResultsChan := make(chan *plugin.QueryProcessorResult)
	wg := sync.WaitGroup{}

	for queryType, queries := range queriesByType {
		wg.Add(1)

		go func(queryType plugin.QueryType, queries plugin.NamedQueries) {
			defer wg.Done()

			if err := queryRunner.RunQueries(queryType, f, sourceCode, queries, queryResultsChan); err != nil {
				msg := fmt.Sprintf("Error running queries for %q: %v", f, err)
				fmt.Printf("%s\n", msg)
				BazelLog.Error(msg)
			}
		}(queryType, queries)
	}

	go func() {
		wg.Wait()
		close(queryResultsChan)
	}()

	// Read the result channel and collect the results
	queryResults := make(plugin.QueryResults, len(queries))
	for result := range queryResultsChan {
		queryResults[result.Key] = result.Result
	}

	return queryResults, nil
}

// Collect source files managed by this BUILD and batch them by plugins interested in them.
func (host *GazelleHost) collectSourceFilesByPlugin(cfg *BUILDConfig, args gazelleLanguage.GenerateArgs) (map[plugin.PluginId][]string, map[string][]plugin.PluginId, map[plugin.PluginId]map[string][]string) {
	pluginSourceFiles := make(map[plugin.PluginId][]string, len(cfg.pluginPrepareResults))
	sourceFilePlugins := make(map[string][]plugin.PluginId)
	pluginSourceGroupFiles := make(map[plugin.PluginId]map[string][]string, len(cfg.pluginPrepareResults))

	// Collect source files managed by this BUILD for each plugin.
	common.GazelleWalkDir(args, func(f string) error {
		for pluginId, p := range cfg.pluginPrepareResults {
			foundGroup := false

			// Collect the groups this file belongs to for this plugin.
			for groupId, groupSrcFilters := range p.Sources {
				for _, srcFilter := range groupSrcFilters {
					if srcFilter.Match(f) {
						foundGroup = true

						if pluginSourceGroupFiles[pluginId] == nil {
							pluginSourceGroupFiles[pluginId] = make(map[string][]string)
						}

						pluginSourceGroupFiles[pluginId][groupId] = append(pluginSourceGroupFiles[pluginId][groupId], f)
						break
					}
				}
			}

			// If the file matched any groups, add it to the file+plugin maps.
			if foundGroup {
				pluginSourceFiles[pluginId] = append(pluginSourceFiles[pluginId], f)
				sourceFilePlugins[f] = append(sourceFilePlugins[f], pluginId)
			}
		}

		return nil
	})

	return pluginSourceFiles, sourceFilePlugins, pluginSourceGroupFiles
}

// Let plugins analyze sources and declare their outputs
func (host *GazelleHost) analyzePluginTargetSources(pluginId plugin.PluginId, prep pluginConfig, sources map[string]plugin.TargetSource) {
	eg := errgroup.Group{}
	eg.SetLimit(100)

	for _, src := range sources {
		eg.Go(func() error {
			actx := plugin.NewAnalyzeContext(prep.PrepareContext, &src, host.database)

			err := host.plugins[pluginId].Analyze(actx)
			if err != nil {
				// TODO:
				fmt.Println(fmt.Errorf("analyze failed for %s: %w", pluginId, err))
			}
			return nil
		})
	}

	if err := eg.Wait(); err != nil {
		BazelLog.Errorf("Analyze plugin error: %v", err)
	}
}

// Let plugins declare any targets they want to generate for the target sources.
func (host *GazelleHost) generateTargets(pluginId plugin.PluginId, prep pluginConfig, sources plugin.TargetSources) []plugin.TargetAction {
	ctx := plugin.NewDeclareTargetsContext(
		prep.PrepareContext,
		sources,
		plugin.NewDeclareTargetActions(),
		host.database,
	)

	return host.plugins[pluginId].DeclareTargets(ctx).Actions
}
