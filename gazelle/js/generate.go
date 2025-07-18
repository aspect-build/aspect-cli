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
	"bytes"
	"encoding/gob"
	"fmt"
	"math"
	"os"
	"path"
	"strings"
	"sync"

	gazelle "github.com/aspect-build/aspect-cli/gazelle/common"
	"github.com/aspect-build/aspect-cli/gazelle/common/cache"
	starlark "github.com/aspect-build/aspect-cli/gazelle/common/starlark"
	node "github.com/aspect-build/aspect-cli/gazelle/js/node"
	parser "github.com/aspect-build/aspect-cli/gazelle/js/parser"
	pnpm "github.com/aspect-build/aspect-cli/gazelle/js/pnpm"
	proto "github.com/aspect-build/aspect-cli/gazelle/js/proto"
	"github.com/aspect-build/aspect-cli/gazelle/js/typescript"
	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
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

	configRelExtension = "__aspect_js_rel"

	MaxWorkerCount = 12
)

var tsProjectReflectedConfigAttributes = []string{
	"tsconfig",
	"allow_js",
	"composite",
	"declaration",
	"declaration_dir",
	"declaration_map",
	"source_map",
	"incremental",
	"ts_build_info_file",
	"no_emit",
	"resolve_json_module",
	"preserve_jsx",
	"out_dir",
	"root_dir",
}

func (ts *typeScriptLang) getImportLabel(imp string) *label.Label {
	return ts.fileLabels[imp]
}

// GenerateRules extracts build metadata from source files in a directory.
// GenerateRules is called in each directory where an update is requested
// in depth-first post-order.
func (ts *typeScriptLang) GenerateRules(args language.GenerateArgs) language.GenerateResult {
	// TODO: move to common location or fix/patch a feature in gazelle
	args.Config.Exts[configRelExtension] = args.Rel

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

func (ts *typeScriptLang) tsPackageInfoToRelsToIndex(cfg *JsGazelleConfig, args language.GenerateArgs, info *TsProjectInfo) []string {
	i := []string{
		// Might be an npm package reference
		cfg.PnpmLockRel(),
	}

	for it := info.imports.Iterator(); it.Next(); {
		impt := it.Value().(ImportStatement)

		// Might be a direct import of a file or dir
		i = append(i, impt.Imp)

		// Might require tsconfig path expansion (rootDir[s], paths etc.)
		i = append(i, ts.tsconfig.ExpandPaths(impt.SourcePath, impt.Imp)...)
	}

	return i
}

func (ts *typeScriptLang) addSourceRules(cfg *JsGazelleConfig, args language.GenerateArgs, result *language.GenerateResult) {
	tsconfigRel, tsconfig := ts.tsconfig.FindConfig(args.Rel)

	// Create a set of source and generated source files per target.
	sourceFileGroups := treemap.NewWithStringComparator()
	generatedFileGroups := treemap.NewWithStringComparator()

	// Collect data files which *may* be added to a target if imported within the sources.
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

	// Util for adding a file to a source group or the data files.
	processPotentialSourceFile := func(groups *treemap.Map, file string) {
		fileExt := path.Ext(file)
		if isSourceFileExt(fileExt) {
			if target := cfg.GetFileSourceTarget(file, tsconfigRootDir); target != nil {
				// Source files belonging to a target group.
				BazelLog.Tracef("add '%s' src '%s/%s'", target.name, args.Rel, file)

				groupFiles, _ := groups.Get(target.name)
				if groupFiles == nil {
					groupFiles = treeset.NewWithStringComparator()
					groups.Put(target.name, groupFiles)
				}
				groupFiles.(*treeset.Set).Add(file)
			} else {
				// Source files with no group, but may still be considered "data"
				// of other source-importing targets such as npm package targets.
				BazelLog.Tracef("add src data file '%s/%s'", args.Rel, file)

				dataFiles.Add(file)
			}
		}

		// Not collect by any target group, but still collect as a data file.
		if isDataFileExt(fileExt) {
			dataFiles.Add(file)
		}
	}

	// Collect source files.
	collectErr := gazelle.GazelleWalkDir(args, func(file string) error {
		processPotentialSourceFile(sourceFileGroups, file)
		return nil
	})
	if collectErr != nil {
		BazelLog.Errorf("Source collection error: %v\n", collectErr)
		return
	}

	// Collect generated files.
	for _, file := range args.GenFiles {
		processPotentialSourceFile(generatedFileGroups, file)
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

		var ruleSrcs, ruleGenSrcs *treeset.Set

		// If the rule has it's own custom list of sources then parse and use that list.
		if existing := gazelle.GetFileRuleByName(args, ruleName); existing != nil && sourceRuleKinds.Contains(existing.Kind()) && starlark.IsCustomSrcs(existing.Attr("srcs")) {
			customSrcs, err := starlark.ExpandSrcs(args.Config.RepoRoot, args.Rel, args.RegularFiles, existing.Attr("srcs"))
			if err != nil {
				BazelLog.Infof("Failed to expand custom srcs %s:%s - %v", args.Rel, existing.Name(), err)
			}

			if customSrcs != nil {
				ruleSrcs = treeset.NewWithStringComparator()
				for _, src := range customSrcs {
					ruleSrcs.Add(src)
				}
			}
		} else {
			if srcs, hasSrcs := sourceFileGroups.Get(group.name); hasSrcs {
				ruleSrcs = srcs.(*treeset.Set)
			}
			if genSrcs, hasGenSrcs := generatedFileGroups.Get(group.name); hasGenSrcs {
				ruleGenSrcs = genSrcs.(*treeset.Set)
			}
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
				ruleGenSrcs,
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

	parserCache := cache.Get(args.Config)
	packageImports, _, err := parserCache.LoadOrStoreFile(args.Config.RepoRoot, packageJsonPath, "parsePackageJsonImports", func(path string, content []byte) (any, error) {
		return node.ParsePackageJsonImports(bytes.NewReader(content))
	})
	if err != nil {
		BazelLog.Warnf("Failed to parse %q imports: %e", packageJsonPath, err)
	}

	for _, impt := range packageImports.([]string) {
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

	npmPackageVisibility := "//:__pkg__"
	if lockDir := path.Dir(cfg.PnpmLockfile()); lockDir != "." {
		npmPackageVisibility = fmt.Sprintf("//%s:__pkg__", lockDir)
	}

	npmPackage := rule.NewRule(packageTargetKind, packageTargetName)
	npmPackage.SetPrivateAttr("ts_project_info", &npmPackageInfo.TsProjectInfo)
	npmPackage.SetAttr("srcs", npmPackageInfo.sources.Values())
	npmPackage.SetAttr("visibility", []string{npmPackageVisibility})

	result.Gen = append(result.Gen, npmPackage)
	result.Imports = append(result.Imports, npmPackageInfo)
	result.RelsToIndex = append(result.RelsToIndex, ts.tsPackageInfoToRelsToIndex(cfg, args, &npmPackageInfo.TsProjectInfo)...)

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
	tsconfigRule.SetAttr("visibility", []string{":__subpackages__"})

	result.Gen = append(result.Gen, tsconfigRule)
	result.Imports = append(result.Imports, imports)
	result.RelsToIndex = append(result.RelsToIndex, ts.tsPackageInfoToRelsToIndex(cfg, args, imports)...)
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
		if !cfg.IsImportIgnored(t) {
			imports = append(imports, ImportStatement{
				ImportSpec: resolve.ImportSpec{
					Lang: LanguageName,
					Imp:  t,
				},
				ImportPath: t,
				SourcePath: SourcePath,
				TypesOnly:  true,
			})
		}
	}

	for _, reference := range tsconfig.References {
		referenceFile := cfg.tsconfigName
		referenceDir := "."
		if strings.HasSuffix(reference, ".json") {
			referenceFile = reference
		} else {
			referenceDir = reference
		}

		imports = append(imports, ImportStatement{
			ImportSpec: resolve.ImportSpec{
				Lang: LanguageName,
				Imp:  path.Join(referenceDir, referenceFile),
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
	result.RelsToIndex = append(result.RelsToIndex, ts.tsPackageInfoToRelsToIndex(cfg, args, imports)...)

	BazelLog.Infof("add rule '%s' '%s:%s'", tsProtoLibrary.Kind(), args.Rel, tsProtoLibrary.Name())
}

func hasTranspiledSources(sourceFiles *treeset.Set) bool {
	return sourceFiles.Any(func(_ int, f any) bool {
		return isTranspiledSourceFileType(f.(string))
	})
}

func (ts *typeScriptLang) addProjectRule(cfg *JsGazelleConfig, tsconfigRel string, tsconfig *typescript.TsConfig, args language.GenerateArgs, group *TargetGroup, targetName string, sourceFiles, genFiles, dataFiles *treeset.Set, result *language.GenerateResult) (*rule.Rule, error) {
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
	sourceFiles.Each(func(_ int, f any) { info.sources.Add(f.(string)) })
	if genFiles != nil {
		genFiles.Each(func(_ int, f any) { info.sources.Add(f.(string)) })
	}

	// Parse source files, do not parse generated files that are not source files.
	for result := range ts.parseFiles(cfg, args, sourceFiles) {
		if result.Error != nil {
			fmt.Printf("%s:\n%s\n\n", result.SourcePath, result.Error)
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
	dataFileWorkspacePaths := make(map[string]string, dataFiles.Size())
	for it := dataFiles.Iterator(); it.Next(); {
		dataFile := it.Value().(string)
		dataFileWorkspacePaths[path.Join(args.Rel, dataFile)] = dataFile
	}

	// Add any imported data files as sources.
	for it := info.imports.Iterator(); it.Next(); {
		importStatement := it.Value().(ImportStatement)
		workspacePath := importStatement.Imp

		// If the imported path is a file that can be compiled as ts source
		// then add it to the importedDataFiles to be included in the srcs.
		// Remove it from the dataFiles to signify that it is now a "source" file
		// owned by this target.
		if dataFile, ok := dataFileWorkspacePaths[workspacePath]; ok {
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

	// Manage the ts_project(isolated_typecheck) attribute
	if ruleKind == TsProjectKind {
		if tsconfig != nil && tsconfig.IsolatedDeclarations != nil {
			// Assign if specified in the tsconfig
			sourceRule.SetAttr("isolated_typecheck", *tsconfig.IsolatedDeclarations)
		}
	} else {
		sourceRule.DelAttr("isolated_typecheck")
	}

	// If the rule kind is not a ts_project rule then delete all tsconfig related attributes.
	// Delete from the existing rule if it exists to bypass any merge/#keep logic related to ts_project.
	if ruleKind != TsProjectKind {
		deleteFrom := sourceRule
		if existing != nil {
			deleteFrom = existing
		}
		for _, attr := range tsProjectReflectedConfigAttributes {
			deleteFrom.DelAttr(attr)
		}
	} else if cfg.GetTsConfigGenerationEnabled() {
		// If generating ts_config() targets also set the ts_project(tsconfig) and related attributes
		// unless they have been explicitly opted out of being reflected.

		if !cfg.IsTsConfigIgnored("allow_js") {
			if tsconfig != nil {
				tsconfigLabel := label.New("", tsconfigRel, cfg.RenderTsConfigName(tsconfig.ConfigName))
				tsconfigLabel = tsconfigLabel.Rel("", args.Rel)

				sourceRule.SetAttr("tsconfig", tsconfigLabel.BzlExpr())
			} else {
				sourceRule.DelAttr("tsconfig")
			}
		}

		// Reflect the tsconfig allowJs in the ts_project rule
		if !cfg.IsTsConfigIgnored("allow_js") {
			if tsconfig != nil && tsconfig.AllowJs != nil {
				sourceRule.SetAttr("allow_js", *tsconfig.AllowJs)
			} else {
				sourceRule.DelAttr("allow_js")
			}
		}

		// Reflect the tsconfig composite in the ts_project rule
		if !cfg.IsTsConfigIgnored("composite") {
			if tsconfig != nil && tsconfig.Composite != nil {
				sourceRule.SetAttr("composite", *tsconfig.Composite)
			} else {
				sourceRule.DelAttr("composite")
			}
		}

		// Reflect the tsconfig declaration in the ts_project rule
		if !cfg.IsTsConfigIgnored("declaration") {
			if tsconfig != nil && tsconfig.Declaration != nil {
				sourceRule.SetAttr("declaration", *tsconfig.Declaration)
			} else {
				sourceRule.DelAttr("declaration")
			}
		}

		// Reflect the tsconfig declarationMap in the ts_project rule
		if !cfg.IsTsConfigIgnored("declaration_map") {
			if tsconfig != nil && tsconfig.DeclarationMap != nil {
				sourceRule.SetAttr("declaration_map", *tsconfig.DeclarationMap)
			} else {
				sourceRule.DelAttr("declaration_map")
			}
		}

		// Reflect the tsconfig emitDeclarationOnly in the ts_project rule
		if !cfg.IsTsConfigIgnored("emit_declaration_only") {
			if tsconfig != nil && tsconfig.DeclarationOnly != nil {
				sourceRule.SetAttr("emit_declaration_only", *tsconfig.DeclarationOnly)
			} else {
				sourceRule.DelAttr("emit_declaration_only")
			}
		}

		// Reflect the tsconfig sourceMap in the ts_project rule
		if !cfg.IsTsConfigIgnored("source_map") {
			if tsconfig != nil && tsconfig.SourceMap != nil {
				sourceRule.SetAttr("source_map", *tsconfig.SourceMap)
			} else {
				sourceRule.DelAttr("source_map")
			}
		}

		// Reflect the tsconfig incremental in the ts_project rule
		if !cfg.IsTsConfigIgnored("incremental") {
			if tsconfig != nil && tsconfig.Incremental != nil {
				sourceRule.SetAttr("incremental", *tsconfig.Incremental)
			} else {
				sourceRule.DelAttr("incremental")
			}
		}

		// Reflect the tsconfig tsBuildInfoFile in the ts_project rule
		if !cfg.IsTsConfigIgnored("ts_build_info_file") {
			if tsconfig != nil && tsconfig.TsBuildInfoFile != "" {
				sourceRule.SetAttr("ts_build_info_file", tsconfig.TsBuildInfoFile)
			} else {
				sourceRule.DelAttr("ts_build_info_file")
			}
		}

		// Reflect the tsconfig noEmit in the ts_project rule
		if !cfg.IsTsConfigIgnored("no_emit") {
			if tsconfig != nil && tsconfig.NoEmit != nil {
				sourceRule.SetAttr("no_emit", *tsconfig.NoEmit)
			} else {
				sourceRule.DelAttr("no_emit")
			}
		}

		// Reflect the tsconfig resolveJsonModule in the ts_project rule
		if !cfg.IsTsConfigIgnored("resolve_json_module") {
			if tsconfig != nil && tsconfig.ResolveJsonModule != nil {
				sourceRule.SetAttr("resolve_json_module", *tsconfig.ResolveJsonModule)
			} else {
				sourceRule.DelAttr("resolve_json_module")
			}
		}

		// Reflect the tsconfig preserveJsx in the ts_project rule
		if !cfg.IsTsConfigIgnored("preserve_jsx") {
			if tsconfig != nil && tsconfig.Jsx != typescript.JsxNone {
				sourceRule.SetAttr("preserve_jsx", tsconfig.Jsx == typescript.JsxPreserve)
			} else {
				sourceRule.DelAttr("preserve_jsx")
			}
		}

		// Reflect the tsconfig outDir in the ts_project rule
		if !cfg.IsTsConfigIgnored("out_dir") {
			if tsconfig != nil && tsconfig.OutDir != "" && tsconfig.OutDir != "." {
				sourceRule.SetAttr("out_dir", tsconfig.OutDir)
			} else {
				sourceRule.DelAttr("out_dir")
			}
		}

		// Reflect the tsconfig outDir in the ts_project rule
		if tsconfig != nil && tsconfig.DeclarationDir != tsconfig.OutDir {
			sourceRule.SetAttr("declaration_dir", tsconfig.DeclarationDir)
		} else {
			sourceRule.DelAttr("declaration_dir")
		}

		// Reflect the tsconfig rootDir in the ts_project rule
		if !cfg.IsTsConfigIgnored("root_dir") {
			if tsconfig != nil && tsconfig.RootDir != "" && tsconfig.RootDir != "." {
				sourceRule.SetAttr("root_dir", tsconfig.RootDir)
			} else {
				sourceRule.DelAttr("root_dir")
			}
		}
	} else {
		// Otherwise when not generating ts_config() targets assign the existing attribute
		// values to keep them instead of gazelle removing them on "merge".
		if existing != nil {
			for _, attr := range tsProjectReflectedConfigAttributes {
				if !cfg.IsTsConfigIgnored(attr) && existing.Attr(attr) != nil {
					sourceRule.SetAttr(attr, existing.Attr(attr))
				}
			}
		}
	}

	result.Gen = append(result.Gen, sourceRule)
	result.Imports = append(result.Imports, info)
	result.RelsToIndex = append(result.RelsToIndex, ts.tsPackageInfoToRelsToIndex(cfg, args, info)...)

	BazelLog.Infof("add rule '%s' '%s:%s'", sourceRule.Kind(), args.Rel, sourceRule.Name())

	return sourceRule, nil
}

type parseResult struct {
	SourcePath string
	Imports    []ImportStatement
	Modules    []string
	Error      error
}

func (ts *typeScriptLang) collectProtoImports(cfg *JsGazelleConfig, args language.GenerateArgs, sourceFiles []string) []ImportStatement {
	results := make([]ImportStatement, 0)

	for _, sourceFile := range sourceFiles {
		imports, err := proto.GetProtoImports(path.Join(args.Dir, sourceFile))
		if err != nil {
			msg := fmt.Sprintf("Error parsing .proto file %q: %v", sourceFile, err)
			BazelLog.Error(msg)
			fmt.Printf("%s:\n", msg)
			continue
		}

		for _, imp := range imports {
			if proto.IsRulesTsProtoBuiltin(imp) {
				BazelLog.Tracef("Proto import builtin: %q", imp)
				continue
			}

			if cfg.IsImportIgnored(imp) {
				BazelLog.Tracef("Proto import ignored: %q", imp)
				continue
			}

			workspacePath := toImportSpecPath(sourceFile, imp)
			workspacePath = strings.TrimSuffix(workspacePath, ".proto")
			workspacePath = workspacePath + "_pb"

			results = append(results, ImportStatement{
				ImportSpec: resolve.ImportSpec{
					Lang: LanguageName,
					Imp:  workspacePath,
				},
				ImportPath: imp,
				SourcePath: sourceFile,
			})
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
	parseResults, err := parseSourceFile(config, rootDir, sourcePath)

	result := parseResult{
		SourcePath: sourcePath,
		Error:      err,
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
func parseSourceFile(config *config.Config, rootDir, filePath string) (parser.ParseResult, error) {
	BazelLog.Tracef("ParseImports(%s): %s", LanguageName, filePath)

	parserCache := cache.Get(config)

	var p parser.ParseResult
	r, _, err := parserCache.LoadOrStoreFile(rootDir, filePath, "js.ParseSource", func(filePath string, content []byte) (any, error) {
		return parser.ParseSource(filePath, content)
	})

	if r != nil {
		p = r.(parser.ParseResult)
	}

	return p, err
}

func init() {
	// TODO: don't expose 'gob' cache serialization here
	gob.Register(parser.ParseResult{})
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
func (ts *typeScriptLang) addPnpmLockfile(c *config.Config, cfg *JsGazelleConfig, lockfileRel string) {
	BazelLog.Infof("pnpm add %q", lockfileRel)

	parsedCache := cache.Get(c)
	parsedLockfile, _, readErr := parsedCache.LoadOrStoreFile(c.RepoRoot, lockfileRel, "pnpm.ParsePnpmLockFile", func(filePath string, content []byte) (any, error) {
		return pnpm.ParsePnpmLockFileDependencies(content)
	})
	if readErr != nil {
		BazelLog.Fatalf("failed to read lockfile %q: %v", lockfileRel, readErr)
	}

	pnpmWorkspace := ts.pnpmProjects.NewWorkspace(lockfileRel)

	for project, packages := range parsedLockfile.(pnpm.WorkspacePackageVersionMap) {
		BazelLog.Debugf("pnpm add %q: project %q ", lockfileRel, project)

		pnpmProject := pnpmWorkspace.AddProject(project)

		for pkg, version := range packages {
			BazelLog.Tracef("pnpm add %q: project %q: package: %q", lockfileRel, project, pkg)

			pnpmProject.AddPackage(pkg, version, &label.Label{
				Repo:     c.RepoName,
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
		return path.Join(importFrom, "..", importPath)
	}

	// URLs of any protocol
	if strings.Contains(importPath, "://") {
		return importPath
	}

	// Non-relative imports such as packages, paths depending on `rootDirs` etc.
	// Clean any extra . / .. etc
	return path.Clean(importPath)
}
