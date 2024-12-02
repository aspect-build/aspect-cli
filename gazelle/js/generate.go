/*
 * Copyright 2023 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package gazelle

import (
	"crypto"
	"encoding/gob"
	"encoding/hex"
	"fmt"
	"math"
	"os"
	"path"
	"strings"
	"sync"

	gazelle "aspect.build/cli/gazelle/common"
	"aspect.build/cli/gazelle/common/cache"
	starlark "aspect.build/cli/gazelle/common/starlark"
	node "aspect.build/cli/gazelle/js/node"
	parser "aspect.build/cli/gazelle/js/parser"
	pnpm "aspect.build/cli/gazelle/js/pnpm"
	proto "aspect.build/cli/gazelle/js/proto"
	"aspect.build/cli/gazelle/js/typescript"
	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/bazelbuild/bazel-gazelle/resolve"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"github.com/emirpasic/gods/maps/treemap"
	"github.com/emirpasic/gods/sets/treeset"
)

const (
	// The filename (with any of the TS extensions) imported when importing a directory.
	IndexFileName      = "index"
	SlashIndexFileName = "/" + IndexFileName

	NpmPackageFilename = "package.json"

	DefaultRootTargetName = "root"

	MaxWorkerCount = 12
)

func (ts *typeScriptLang) getImportLabel(imp string) *label.Label {
	return ts.fileLabels[imp]
}

// GenerateRules extracts build metadata from source files in a directory.
// GenerateRules is called in each directory where an update is requested
// in depth-first post-order.
func (ts *typeScriptLang) GenerateRules(args language.GenerateArgs) language.GenerateResult {
	cfg := args.Config.Exts[LanguageName].(*JsGazelleConfig)

	// Collect any labels that could be imported
	ts.collectFileLabels(args)

	// When we return empty, we mean that we don't generate anything, but this
	// still triggers the indexing for all the TypeScript targets in this package.
	if !cfg.GenerationEnabled() {
		BazelLog.Tracef("GenerateRules(%s) disabled: %s", LanguageName, args.Rel)
		return language.GenerateResult{}
	}

	BazelLog.Tracef("GenerateRules(%s): %s", LanguageName, args.Rel)

	var result language.GenerateResult

	ts.addPackageRules(cfg, args, &result)
	ts.addSourceRules(cfg, args, &result)

	if cfg.GetTsConfigGenerationEnabled() {
		ts.addTsConfigRules(cfg, args, &result)
	}

	if cfg.ProtoGenerationEnabled() {
		ts.addTsProtoRules(cfg, args, &result)
	}

	return result
}

func (ts *typeScriptLang) addSourceRules(cfg *JsGazelleConfig, args language.GenerateArgs, result *language.GenerateResult) {
	tsconfigRel, tsconfig := ts.tsconfig.FindConfig(args.Rel)

	// Create a set of source files per target.
	sourceFileGroups := treemap.NewWithStringComparator()
	for _, group := range cfg.GetSourceTargets() {
		sourceFileGroups.Put(group.name, treeset.NewWithStringComparator())
	}
	dataFiles := treeset.NewWithStringComparator()

	// Calculate the tsconfig rootDir relative to the current directory being walked
	tsconfigRootDir := "."
	if tsconfig != nil {
		tsconfigRootDir = path.Join(tsconfigRel, tsconfig.RootDir)

		// Ignore rootDirs not within args.Rel
		if args.Rel != "" && !strings.HasPrefix(tsconfigRootDir, args.Rel+"/") {
			tsconfigRootDir = "."
		} else if args.Rel != "" {
			// Make the rootDir relative to the current directory being walked
			tsconfigRootDir = tsconfigRootDir[len(args.Rel)+1:]
		}
	}

	// Collect all source files.
	collectErr := gazelle.GazelleWalkDir(args, func(file string) error {
		fileExt := path.Ext(file)
		if isSourceFileExt(fileExt) {
			if target := cfg.GetFileSourceTarget(file, tsconfigRootDir); target != nil {
				BazelLog.Tracef("add '%s' src '%s/%s'", target.name, args.Rel, file)

				groupFiles, _ := sourceFileGroups.Get(target.name)
				groupFiles.(*treeset.Set).Add(file)
				return nil
			}
			BazelLog.Tracef("Skip src '%s'", file)
		}

		// Not collect by any target group, but still collect as a data file.
		if isDataFileExt(fileExt) {
			dataFiles.Add(file)
		}
		return nil
	})
	if collectErr != nil {
		BazelLog.Errorf("Source collection error: %v\n", collectErr)
		return
	}

	// Determine if this is a pnpm project and if a package target should be generated.
	isPnpmPackage := ts.pnpmProjects.IsProject(args.Rel)
	hasPackageTarget := isPnpmPackage && (cfg.GetNpmPackageGenerationMode() == NpmPackageEnabledMode || cfg.GetNpmPackageGenerationMode() == NpmPackageReferencedMode && ts.pnpmProjects.IsReferenced(args.Rel))

	// The package/directory name variable value used to render the target names.
	packageName := gazelle.ToDefaultTargetName(args, DefaultRootTargetName)

	// Create rules for each target group.
	sourceRules := treemap.NewWithStringComparator()
	for _, group := range cfg.GetSourceTargets() {
		// The project rule name. Can be configured to map to a different name.
		ruleName := cfg.RenderSourceTargetName(group.name, packageName, hasPackageTarget)

		var ruleSrcs *treeset.Set

		// If the rule has it's own custom list of sources then parse and use that list.
		if existing := gazelle.GetFileRuleByName(args, ruleName); existing != nil && sourceRuleKinds.Contains(existing.Kind()) && starlark.IsCustomSrcs(existing.Attr("srcs")) {
			customSrcs, err := starlark.ExpandSrcs(args.Config.RepoRoot, args.Rel, existing.Attr("srcs"))
			if err != nil {
				BazelLog.Infof("Failed to expand custom srcs %s:%s - %v", args.Rel, existing.Name(), err)
			}

			if customSrcs != nil {
				ruleSrcs = treeset.NewWithStringComparator()
				for _, src := range customSrcs {
					ruleSrcs.Add(src)
				}
			}
		} else if srcs, hasSrcs := sourceFileGroups.Get(group.name); hasSrcs {
			ruleSrcs = srcs.(*treeset.Set)
		}

		if ruleSrcs == nil || ruleSrcs.Empty() {
			// No sources for this source group. Remove the rule if it exists.
			gazelle.RemoveRule(args, ruleName, sourceRuleKinds, result)
		} else {
			// Add or edit/merge a rule for this source group.
			srcRule, srcGenErr := ts.addProjectRule(
				cfg,
				tsconfigRel,
				tsconfig,
				args,
				group,
				ruleName,
				ruleSrcs,
				dataFiles,
				result,
			)
			if srcGenErr != nil {
				fmt.Fprintf(os.Stderr, "Source rule generation error: %v\n", srcGenErr)
				os.Exit(1)
			}

			sourceRules.Put(group.name, srcRule)
		}
	}

	// If this is a package wrap the main ts_project() rule with npm_package()
	if hasPackageTarget {
		// Add the primary source rule by default if it exists
		var srcLabel *label.Label
		if srcRule, _ := sourceRules.Get(DefaultLibraryName); srcRule != nil {
			srcLabel = &label.Label{
				Name:     srcRule.(*rule.Rule).Name(),
				Repo:     args.Config.RepoName,
				Pkg:      args.Rel,
				Relative: true,
			}
		}

		ts.addPackageRule(cfg, args, packageName, dataFiles, srcLabel, result)
	}
}

func (ts *typeScriptLang) addPackageRule(cfg *JsGazelleConfig, args language.GenerateArgs, packageName string, dataFiles *treeset.Set, srcLabel *label.Label, result *language.GenerateResult) {
	npmPackageInfo := newTsPackageInfo(srcLabel)

	packageJsonPath := path.Join(args.Rel, NpmPackageFilename)
	packageImports, err := node.ParsePackageJsonImportsFile(args.Config.RepoRoot, packageJsonPath)
	if err != nil {
		BazelLog.Warnf("Failed to parse %q imports: %e", packageJsonPath, err)
	}

	for _, impt := range packageImports {
		if cfg.IsImportIgnored(impt) {
			continue
		}

		if dataFiles.Contains(impt) {
			npmPackageInfo.sources.Add(impt)
		} else {
			if strings.Contains(impt, "*") {
				BazelLog.Debugf("Wildcard import %q in %q not supported", impt, packageJsonPath)
				continue
			}

			npmPackageInfo.imports.Add(ImportStatement{
				ImportSpec: resolve.ImportSpec{
					Lang: LanguageName,
					Imp:  path.Join(args.Rel, impt),
				},
				ImportPath: impt,
				SourcePath: packageJsonPath,

				// Set as optional while package.json imports are experimental
				Optional: true,
			})
		}
	}

	// Add the package.json if not in the src
	// TODO: why not always add it?
	// TODO: declare import on it instead if it's in another rule?
	if dataFiles.Contains(NpmPackageFilename) {
		dataFiles.Remove(NpmPackageFilename)
		npmPackageInfo.sources.Add(NpmPackageFilename)
	}

	packageTargetName := cfg.RenderNpmPackageTargetName(packageName)
	packageTargetKind := NpmPackageKind
	if cfg.packageTargetKind == PackageTargetKind_Library {
		packageTargetKind = JsLibraryKind
	}

	npmPackage := rule.NewRule(packageTargetKind, packageTargetName)
	npmPackage.SetAttr("srcs", npmPackageInfo.sources.Values())
	npmPackage.SetAttr("visibility", []string{rule.CheckInternalVisibility(cfg.rel, "//visibility:public")})

	result.Gen = append(result.Gen, npmPackage)
	result.Imports = append(result.Imports, npmPackageInfo)

	BazelLog.Infof("add rule '%s' '%s:%s'", cfg.packageTargetKind, args.Rel, packageTargetName)
}

func (ts *typeScriptLang) addTsConfigRules(cfg *JsGazelleConfig, args language.GenerateArgs, result *language.GenerateResult) {
	tsconfig := ts.tsconfig.GetTsConfigFile(args.Rel)
	if tsconfig == nil {
		return
	}

	imports := newTsProjectInfo()
	for _, impt := range ts.collectTsConfigImports(cfg, args, tsconfig) {
		imports.AddImport(impt)
	}

	tsconfigName := cfg.RenderTsConfigName(tsconfig.ConfigName)
	tsconfigRule := rule.NewRule(TsConfigKind, tsconfigName)
	tsconfigRule.SetAttr("src", tsconfig.ConfigName)

	result.Gen = append(result.Gen, tsconfigRule)
	result.Imports = append(result.Imports, imports)
}

func (ts *typeScriptLang) collectTsConfigImports(cfg *JsGazelleConfig, args language.GenerateArgs, tsconfig *typescript.TsConfig) []ImportStatement {
	imports := make([]ImportStatement, 0)

	SourcePath := path.Join(tsconfig.ConfigDir, tsconfig.ConfigName)

	if tsconfig.Extends != "" {
		if !cfg.IsImportIgnored(tsconfig.Extends) {
			imports = append(imports, ImportStatement{
				ImportSpec: resolve.ImportSpec{
					Lang: LanguageName,
					Imp:  toImportSpecPath(SourcePath, tsconfig.Extends),
				},
				ImportPath: tsconfig.Extends,
				SourcePath: SourcePath,
			})
		}
	}

	for _, t := range tsconfig.Types {
		if typesImport := toAtTypesPackage(t); !cfg.IsImportIgnored(typesImport) {
			imports = append(imports, ImportStatement{
				ImportSpec: resolve.ImportSpec{
					Lang: LanguageName,
					Imp:  typesImport,
				},
				ImportPath: t,
				SourcePath: SourcePath,
			})
		}
	}

	for _, reference := range tsconfig.References {
		// TODO: how do we know the referenced tsconfig filename?
		referenceFile := cfg.tsconfigName

		imports = append(imports, ImportStatement{
			ImportSpec: resolve.ImportSpec{
				Lang: LanguageName,
				Imp:  path.Join(reference, referenceFile),
			},
			ImportPath: reference,
			SourcePath: SourcePath,
		})
	}

	return imports
}

func (ts *typeScriptLang) addTsProtoRules(cfg *JsGazelleConfig, args language.GenerateArgs, result *language.GenerateResult) {
	protoLibraries, emptyLibraries := proto.GetProtoLibraries(args, result)

	// Generate one ts_proto_library() per proto_library()
	for _, protoLibrary := range protoLibraries {
		ruleName := cfg.RenderTsProtoLibraryName(protoLibrary.Name())
		ts.addTsProtoRule(cfg, args, protoLibrary, ruleName, result)
	}

	// Remove any ts_proto_library() targets associated with now-empty proto_library() targets
	for _, emptyLibrary := range emptyLibraries {
		ruleName := cfg.RenderTsProtoLibraryName(emptyLibrary.Name())
		gazelle.RemoveRule(args, ruleName, sourceRuleKinds, result)
	}
}

func (ts *typeScriptLang) addTsProtoRule(cfg *JsGazelleConfig, args language.GenerateArgs, protoLibrary *rule.Rule, ruleName string, result *language.GenerateResult) {
	protoRuleLabel := label.New("", args.Rel, protoLibrary.Name())
	protoRuleLabelStr := protoRuleLabel.Rel("", args.Rel)

	tsProtoLibrary := rule.NewRule(TsProtoLibraryKind, ruleName)
	tsProtoLibrary.SetAttr("proto", protoRuleLabelStr.String())

	node_modules := ts.pnpmProjects.GetProject(args.Rel)
	if node_modules != nil {
		node_modulesLabel := label.New("", node_modules.Pkg(), cfg.npmLinkAllTargetName)
		node_modulesLabelStr := node_modulesLabel.Rel("", args.Rel)
		tsProtoLibrary.SetAttr("node_modules", node_modulesLabelStr.String())
	}

	sourceFiles := protoLibrary.AttrStrings("srcs")

	// Persist the proto_library(srcs)
	tsProtoLibrary.SetPrivateAttr("proto_library_srcs", sourceFiles)

	imports := newTsProjectInfo()

	for _, impt := range ts.collectProtoImports(cfg, args, sourceFiles) {
		imports.AddImport(impt)
	}

	result.Gen = append(result.Gen, tsProtoLibrary)
	result.Imports = append(result.Imports, imports)

	BazelLog.Infof("add rule '%s' '%s:%s'", tsProtoLibrary.Kind(), args.Rel, tsProtoLibrary.Name())
}

func hasTranspiledSources(sourceFiles *treeset.Set) bool {
	for _, f := range sourceFiles.Values() {
		if isTranspiledSourceFileType(f.(string)) {
			return true
		}
	}

	return false
}

func (ts *typeScriptLang) addProjectRule(cfg *JsGazelleConfig, tsconfigRel string, tsconfig *typescript.TsConfig, args language.GenerateArgs, group *TargetGroup, targetName string, sourceFiles, dataFiles *treeset.Set, result *language.GenerateResult) (*rule.Rule, error) {
	// Check for name-collisions with the rule being generated.
	colError := gazelle.CheckCollisionErrors(targetName, TsProjectKind, sourceRuleKinds, args)
	if colError != nil {
		return nil, fmt.Errorf("%v "+
			"Use the '# aspect:%s' directive to change the naming convention.\n\n"+
			"For example:\n"+
			"\t# aspect:%s {dirname}_js\n"+
			"\t# aspect:%s {dirname}_js_tests",
			colError.Error(),
			Directive_LibraryNamingConvention,
			Directive_LibraryNamingConvention,
			Directive_TestsNamingConvention,
		)
	}

	// Project data combined from all files.
	info := newTsProjectInfo()
	info.sources.Add(sourceFiles.Values()...)

	for result := range ts.parseFiles(cfg, args, info.sources) {
		if len(result.Errors) > 0 {
			fmt.Printf("%s:\n", result.SourcePath)
			for _, err := range result.Errors {
				fmt.Printf("%s\n", err)
			}
			fmt.Println()
		}

		for _, sourceImport := range result.Imports {
			info.AddImport(sourceImport)
		}

		for _, sourceModule := range result.Modules {
			ts.addModuleDeclaration(sourceModule, &label.Label{
				Name:     targetName,
				Repo:     args.Config.RepoName,
				Pkg:      args.Rel,
				Relative: false,
			})
		}
	}

	// tsconfig 'jsx' options implying a dependency on react
	if tsconfig != nil && tsconfig.Jsx.IsReact() && info.HasTsx() {
		info.AddImport(ImportStatement{
			ImportSpec: resolve.ImportSpec{
				Lang: LanguageName,
				Imp:  "react",
			},
			ImportPath: string(tsconfig.Jsx),
			SourcePath: path.Join(tsconfig.ConfigDir, tsconfig.ConfigName),
		})
	}

	// Data file lookup map. Workspace path => local path
	dataFileWorkspacePaths := treemap.NewWithStringComparator()
	for _, dataFile := range dataFiles.Values() {
		dataFileWorkspacePaths.Put(path.Join(args.Rel, dataFile.(string)), dataFile)
	}

	// Add any imported data files as sources.
	for _, importStatement := range info.imports.Values() {
		workspacePath := importStatement.(ImportStatement).Imp

		// If the imported path is a file that can be compiled as ts source
		// then add it to the importedDataFiles to be included in the srcs.
		// Remove it from the dataFiles to signify that it is now a "source" file
		// owned by this target.
		if dataFile, _ := dataFileWorkspacePaths.Get(workspacePath); dataFile != nil {
			info.sources.Add(dataFile)
			dataFiles.Remove(dataFile)
		}
	}

	// A rule of the same name might already exist
	existing := gazelle.GetFileRuleByName(args, targetName)

	ruleKind := TsProjectKind
	if !hasTranspiledSources(info.sources) {
		ruleKind = JsLibraryKind
	}
	sourceRule := rule.NewRule(ruleKind, targetName)

	// TODO: this seems like a hack...
	// Gazelle should support new rules changing the type of existing rules?
	if existing != nil && existing.Kind() != ruleKind {
		existing.SetKind(ruleKind)
	}

	sourceRule.SetPrivateAttr("ts_project_info", info)
	sourceRule.SetAttr("srcs", info.sources.Values())

	if group.testonly {
		sourceRule.SetAttr("testonly", true)
	}

	if len(group.visibility) > 0 {
		sourceRule.SetAttr("visibility", group.visibility)
	}

	// If generating ts_config() targets also set the ts_project(tsconfig) and related attributes
	if cfg.GetTsConfigGenerationEnabled() {
		if tsconfig != nil && ruleKind == TsProjectKind {
			// Set the tsconfig and related attributes if a tsconfig file is found for this target
			// and the target is a ts_project rule.
			tsconfigLabel := label.New("", tsconfigRel, cfg.RenderTsConfigName(tsconfig.ConfigName))
			tsconfigLabel = tsconfigLabel.Rel("", args.Rel)

			sourceRule.SetAttr("tsconfig", tsconfigLabel.BzlExpr())

			// Reflect the tsconfig allow_js in the ts_project rule
			if tsconfig.AllowJs != nil {
				sourceRule.SetAttr("allow_js", *tsconfig.AllowJs)
			} else if existing != nil {
				existing.DelAttr("allow_js")
			}

			// Reflect the tsconfig declaration in the ts_project rule
			if tsconfig.Declaration != nil {
				sourceRule.SetAttr("declaration", *tsconfig.Declaration)
			} else if existing != nil {
				existing.DelAttr("declaration")
			}

			// Reflect the tsconfig declaration_map in the ts_project rule
			if tsconfig.DeclarationMap != nil {
				sourceRule.SetAttr("declaration_map", *tsconfig.DeclarationMap)
			} else if existing != nil {
				existing.DelAttr("declaration_map")
			}

			// Reflect the tsconfig declaration in the ts_project rule
			if tsconfig.SourceMap != nil {
				sourceRule.SetAttr("source_map", *tsconfig.SourceMap)
			} else if existing != nil {
				existing.DelAttr("source_map")
			}

			// Reflect the tsconfig resolve_json_module in the ts_project rule
			if tsconfig.ResolveJsonModule != nil {
				sourceRule.SetAttr("resolve_json_module", *tsconfig.ResolveJsonModule)
			} else if existing != nil {
				existing.DelAttr("resolve_json_module")
			}

			// Reflect the tsconfig resolve_json_module in the ts_project rule
			if tsconfig.Jsx != typescript.JsxNone {
				sourceRule.SetAttr("preserve_jsx", tsconfig.Jsx == typescript.JsxPreserve)
			} else if existing != nil {
				existing.DelAttr("preserve_jsx")
			}

			// Reflect the tsconfig out_dir in the ts_project rule
			if tsconfig.OutDir != "" && tsconfig.OutDir != "." {
				sourceRule.SetAttr("out_dir", tsconfig.OutDir)
			} else if existing != nil {
				existing.DelAttr("out_dir")
			}

			// Reflect the tsconfig root_dir in the ts_project rule
			if tsconfig.RootDir != "" && tsconfig.RootDir != "." {
				sourceRule.SetAttr("root_dir", tsconfig.RootDir)
			} else if existing != nil {
				existing.DelAttr("root_dir")
			}
		} else if existing != nil {
			// Clear tsconfig related attributes if no tsconfig is found
			existing.DelAttr("tsconfig")
			existing.DelAttr("allow_js")
			existing.DelAttr("declaration")
			existing.DelAttr("declaration_map")
			existing.DelAttr("out_dir")
			existing.DelAttr("preserve_jsx")
			existing.DelAttr("resolve_json_module")
			existing.DelAttr("source_map")
			existing.DelAttr("root_dir")
		}
	}

	result.Gen = append(result.Gen, sourceRule)
	result.Imports = append(result.Imports, info)

	BazelLog.Infof("add rule '%s' '%s:%s'", sourceRule.Kind(), args.Rel, sourceRule.Name())

	return sourceRule, nil
}

type parseResult struct {
	SourcePath string
	Imports    []ImportStatement
	Modules    []string
	Errors     []error
}

func (ts *typeScriptLang) collectProtoImports(cfg *JsGazelleConfig, args language.GenerateArgs, sourceFiles []string) []ImportStatement {
	results := make([]ImportStatement, 0)

	for _, sourceFile := range sourceFiles {
		imports, err := proto.GetProtoImports(path.Join(args.Rel, sourceFile))
		if err != nil {
			fmt.Printf("%s:\n", sourceFile)
			fmt.Printf("%s\n", err)
			fmt.Println()
		}

		for _, imp := range imports {
			for _, dts := range proto.ToTsImports(imp) {
				results = append(results, ImportStatement{
					ImportSpec: resolve.ImportSpec{
						Lang: LanguageName,
						Imp:  dts,
					},
					ImportPath: imp,
					SourcePath: sourceFile,
				})
			}
		}
	}

	return results
}

func (ts *typeScriptLang) parseFiles(cfg *JsGazelleConfig, args language.GenerateArgs, sourceFiles *treeset.Set) chan parseResult {
	// The channel of all files to parse.
	sourcePathChannel := make(chan string)

	// The channel of parse results.
	resultsChannel := make(chan parseResult)

	// The number of workers. Don't create more workers than necessary.
	workerCount := int(math.Min(MaxWorkerCount, float64(1+sourceFiles.Size()/2)))

	// Start the worker goroutines.
	var wg sync.WaitGroup
	for i := 0; i < workerCount; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()

			for sourcePath := range sourcePathChannel {
				resultsChannel <- ts.collectImports(cfg, args.Config, args.Config.RepoRoot, sourcePath)
			}
		}()
	}

	// Send files to the workers.
	go func() {
		sourceFileChannelIt := sourceFiles.Iterator()
		for sourceFileChannelIt.Next() {
			sourcePathChannel <- path.Join(args.Rel, sourceFileChannelIt.Value().(string))
		}

		close(sourcePathChannel)
	}()

	// Wait for all workers to finish.
	go func() {
		wg.Wait()
		close(resultsChannel)
	}()

	return resultsChannel
}

func (ts *typeScriptLang) collectImports(cfg *JsGazelleConfig, config *config.Config, rootDir, sourcePath string) parseResult {
	parseResults, errs := parseSourceFile(config, rootDir, sourcePath)

	result := parseResult{
		SourcePath: sourcePath,
		Errors:     errs,
		Imports:    make([]ImportStatement, 0, len(parseResults.Imports)),
		Modules:    parseResults.Modules,
	}

	for _, importPath := range parseResults.Imports {
		if cfg.IsImportIgnored(importPath) {
			BazelLog.Tracef("Import ignored: %q", importPath)
			continue
		}

		// The path from the root
		workspacePath := toImportSpecPath(sourcePath, importPath)

		// Record all imports. Maybe local, maybe data, maybe in other BUILD etc.
		result.Imports = append(result.Imports, ImportStatement{
			ImportSpec: resolve.ImportSpec{
				Lang: LanguageName,
				Imp:  workspacePath,
			},
			ImportPath: importPath,
			SourcePath: sourcePath,
		})

		BazelLog.Tracef("Import: %q -> %q (via %q)", sourcePath, workspacePath, importPath)
	}

	return result
}

// Parse the passed file for import statements.
func parseSourceFile(config *config.Config, rootDir, filePath string) (parser.ParseResult, []error) {
	BazelLog.Tracef("ParseImports(%s): %s", LanguageName, filePath)

	content, err := os.ReadFile(path.Join(rootDir, filePath))
	if err != nil {
		return parser.ParseResult{}, []error{err}
	}

	parserCache := cache.Get[parser.ParseResult](config)
	parserCacheKey, parsingCacheable := computeCacheKey(content)
	if parserCache != nil && parsingCacheable {
		if cachedResults, found := parserCache.Load(parserCacheKey); found {
			return cachedResults.(parser.ParseResult), nil
		}
	}

	r, errs := parser.ParseSource(filePath, content)
	if parserCache != nil && parsingCacheable && len(errs) == 0 {
		parserCache.Store(parserCacheKey, r)
	}

	return r, errs
}

func init() {
	// TODO: don't expose 'gob' cache serialization here
	gob.Register(parser.ParseResult{})
}

func computeCacheKey(content []byte) (string, bool) {
	cacheDigest := crypto.MD5.New()

	if _, err := cacheDigest.Write(content); err != nil {
		return "", false
	}

	return hex.EncodeToString(cacheDigest.Sum(nil)), true
}

func (ts *typeScriptLang) addFileLabel(importPath string, label *label.Label) {
	existing := ts.fileLabels[importPath]

	if existing != nil && isDeclarationFileType(existing.Name) {
		// Can not have two imports (such as .js and .d.ts) from different labels
		if isDeclarationFileType(label.Name) && !existing.Equal(*label) {
			BazelLog.Fatalf("Duplicate file label ", importPath, " from ", existing.String(), " and ", label.String())
		}

		// Prefer the non-declaration file
		return
	}

	// Otherwise overwrite the existing non-declaration version
	ts.fileLabels[importPath] = label
}

func (ts *typeScriptLang) addModuleDeclaration(module string, moduleLabel *label.Label) {
	if ts.moduleTypes[module] == nil {
		ts.moduleTypes[module] = make([]*label.Label, 0, 1)
	}

	ts.moduleTypes[module] = append(ts.moduleTypes[module], moduleLabel)
}

// Find names/paths that the given path can be imported as.
func toImportPaths(p string) []string {
	// NOTE: this is invoked extremely frequently so it's important to keep it fast and light.
	// Do not cause unnecessary memory allocations such as splitting or slicing strings.

	// There will most likely be only one import path when a file is already known to be a "source file"
	paths := make([]string, 0, 1)

	pExt := path.Ext(p)
	pNoExt := p[:len(p)-len(pExt)]

	if isDeclarationFileType(p) {
		pNoExt := p[:len(pNoExt)-2]

		// The import of the raw dts file
		paths = append(paths, p)

		// Assume the js extension also exists
		// TODO: don't do that
		paths = append(paths, pNoExt+toJsExt(pExt))

		// Without the dts extension
		if isImplicitSourceFileExt(pExt) {
			paths = append(paths, pNoExt)
		}

		// Directory without the filename
		if strings.HasSuffix(pNoExt, SlashIndexFileName) {
			paths = append(paths, pNoExt[:len(pNoExt)-len(SlashIndexFileName)])
		}
	} else if isTranspiledSourceFileExt(pExt) {
		// The transpiled files extensions
		paths = append(paths, pNoExt+toJsExt(pExt), pNoExt+toDtsExt(pExt))

		// Without the extension if it is implicit
		if isImplicitSourceFileExt(pExt) {
			paths = append(paths, pNoExt)
		}

		// Directory without the filename
		if strings.HasSuffix(pNoExt, SlashIndexFileName) {
			paths = append(paths, pNoExt[:len(pNoExt)-len(SlashIndexFileName)])
		}
	} else if isSourceFileExt(pExt) {
		// The import of the raw file
		paths = append(paths, p)

		// Without the extension if it is implicit
		if isImplicitSourceFileExt(pExt) {
			paths = append(paths, pNoExt)
		}

		// Directory without the filename
		if strings.HasSuffix(pNoExt, SlashIndexFileName) {
			paths = append(paths, pNoExt[:len(pNoExt)-len(SlashIndexFileName)])
		}
	} else if isDataFileExt(pExt) {
		paths = append(paths, p)
	}

	return paths
}

// Collect and persist all possible references to files that can be imported
func (ts *typeScriptLang) collectFileLabels(args language.GenerateArgs) {
	// Generated files from rules such as genrule()
	for _, f := range args.GenFiles {
		// Label referencing that generated file
		genLabel := label.Label{
			Name: f,
			Repo: args.Config.RepoName,
			Pkg:  args.Rel,
		}

		for _, importPath := range toImportPaths(path.Join(args.Rel, f)) {
			ts.addFileLabel(importPath, &genLabel)
		}
	}

	// TODO(jbedard): record other generated non-source files (args.OtherGen, ?)
}

// Add rules representing packages, node_modules etc
func (ts *typeScriptLang) addPackageRules(cfg *JsGazelleConfig, args language.GenerateArgs, result *language.GenerateResult) {
	if ts.pnpmProjects.IsProject(args.Rel) {
		addLinkAllPackagesRule(cfg, args, result)
	}
}

// Add pnpm rules for a pnpm lockfile.
// Collect pnpm projects and project dependencies from the lockfile.
func (ts *typeScriptLang) addPnpmLockfile(cfg *JsGazelleConfig, repoName, repoRoot, lockfileRel string) {
	BazelLog.Infof("pnpm add %q", lockfileRel)

	lockfilePath := path.Join(repoRoot, lockfileRel)

	pnpmWorkspace := ts.pnpmProjects.NewWorkspace(lockfileRel)

	for project, packages := range pnpm.ParsePnpmLockFileDependencies(lockfilePath) {
		BazelLog.Debugf("pnpm add %q: project %q ", lockfileRel, project)

		pnpmProject := pnpmWorkspace.AddProject(project)

		for pkg, version := range packages {
			BazelLog.Tracef("pnpm add %q: project %q: package: %q", lockfileRel, project, pkg)

			pnpmProject.AddPackage(pkg, version, &label.Label{
				Repo:     repoName,
				Pkg:      pnpmProject.Pkg(),
				Name:     path.Join(cfg.npmLinkAllTargetName, pkg),
				Relative: false,
			})
		}
	}
}

func addLinkAllPackagesRule(cfg *JsGazelleConfig, args language.GenerateArgs, result *language.GenerateResult) {
	npmLinkAll := rule.NewRule(NpmLinkAllKind, cfg.npmLinkAllTargetName)

	result.Gen = append(result.Gen, npmLinkAll)
	result.Imports = append(result.Imports, newLinkAllPackagesImports())

	BazelLog.Infof("add rule '%s' '%s:%s'", npmLinkAll.Kind(), args.Rel, npmLinkAll.Name())
}

// If the file is ts-compatible transpiled source code that may contain imports
func isTranspiledSourceFileType(f string) bool {
	return isTranspiledSourceFileExt(path.Ext(f)) && !isDeclarationFileType(f)
}

// If the file extension is one which must be transpiled.
// Note caution must be taken if the file extension originated from a file that
// may already be transpiled to a .d.ts file.
func isTranspiledSourceFileExt(ext string) bool {
	switch ext {
	case ".ts", ".cts", ".mts", ".tsx", ".jsx":
		return true
	default:
		return false
	}
}

// If the file is ts-compatible source code that may contain imports
func isSourceFileExt(ext string) bool {
	switch ext {
	case ".ts", ".cts", ".mts", ".tsx", ".jsx", ".js", ".cjs", ".mjs":
		return true
	default:
		return false
	}
}

// A source file extension that does not explicitly declare itself as cjs or mjs so
// it can be imported as if it is either. Node will decide how to interpret
// it at runtime based on other factors.
func isImplicitSourceFileExt(ext string) bool {
	switch ext {
	case ".ts", ".tsx", ".js", ".jsx":
		return true
	default:
		return false
	}
}

func isTsxFileExt(e string) bool {
	switch e {
	case ".tsx", ".jsx":
		return true
	default:
		return false
	}
}

// Importable declaration files that are not compiled
func isDeclarationFileType(f string) bool {
	return strings.HasSuffix(f, ".d.ts") || strings.HasSuffix(f, ".d.mts") || strings.HasSuffix(f, ".d.cts")
}

// Supported data file extensions that typescript can reference.
func isDataFileExt(e string) bool {
	return e == ".json"
}

func toJsExt(e string) string {
	switch e {
	case ".ts", ".tsx":
		return ".js"
	case ".cts":
		return ".cjs"
	case ".mts":
		return ".mjs"
	case ".jsx":
		return ".js"
	case ".js", ".cjs", ".mjs", ".json":
		return e
	default:
		BazelLog.Errorf("Unknown extension %q", e)
		return ".js"
	}
}

func toDtsExt(e string) string {
	switch e {
	case ".ts", ".tsx":
		return ".d.ts"
	case ".cts":
		return ".d.cts"
	case ".mts":
		return ".d.mts"
	default:
		BazelLog.Errorf("Unknown extension %q", e)
		return ".d.ts"
	}
}

// Normalize the given import statement from a relative path
// to a path relative to the workspace.
func toImportSpecPath(importFrom, importPath string) string {
	// Relative paths
	if importPath[0] == '.' {
		return path.Join(path.Dir(importFrom), importPath)
	}

	// URLs of any protocol
	if strings.Contains(importPath, "://") {
		return importPath
	}

	// Non-relative imports such as packages, paths depending on `rootDirs` etc.
	// Clean any extra . / .. etc
	return path.Clean(importPath)
}
