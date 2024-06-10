package gazelle

import (
	"fmt"
	"math"
	"os"
	"path"
	"path/filepath"
	"strings"
	"sync"

	gazelle "aspect.build/cli/gazelle/common"
	starlark "aspect.build/cli/gazelle/common/starlark"
	"aspect.build/cli/gazelle/js/parser"
	treesitter_parser "aspect.build/cli/gazelle/js/parser/treesitter"
	pnpm "aspect.build/cli/gazelle/js/pnpm"
	proto "aspect.build/cli/gazelle/js/proto"
	"aspect.build/cli/gazelle/js/typescript"
	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/bazelbuild/bazel-gazelle/resolve"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"github.com/emirpasic/gods/maps/treemap"
	"github.com/emirpasic/gods/sets/treeset"
)

const (
	// The filename (with any of the TS extensions) imported when importing a directory.
	IndexFileName = "index"

	NpmPackageFilename = "package.json"

	DefaultRootTargetName = "root"

	MaxWorkerCount = 12
)

func (ts *typeScriptLang) GetImportLabel(imp string) *label.Label {
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
		BazelLog.Tracef("GenerateRules disabled '%s'", args.Rel)
		return language.GenerateResult{}
	}

	// If this directory has not been declared as a bazel package only continue
	// if generating new BUILD files is enabled.
	if cfg.GenerationMode() == GenerationModeNone && !gazelle.IsBazelPackage(args.Config, args.Dir) {
		BazelLog.Tracef("GenerateRules '%s' BUILD creation disabled", args.Rel)
		return language.GenerateResult{}
	}

	BazelLog.Tracef("GenerateRules '%s'", args.Rel)

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
	// Collect all source files.
	sourceFiles, dataFiles, collectErr := ts.collectSourceFiles(cfg, args)
	if collectErr != nil {
		BazelLog.Errorf("Source collection error: %v\n", collectErr)
		return
	}

	// Create a set of source files per target.
	sourceFileGroups := treemap.NewWithStringComparator()
	for _, group := range cfg.GetSourceTargets() {
		sourceFileGroups.Put(group.name, treeset.NewWithStringComparator())
	}

	// A src files into target groups (lib, test, ...custom).
	for _, f := range sourceFiles.Values() {
		// TODO: exclude files which are included in custom targets via #keep

		file := f.(string)
		if target := cfg.GetSourceTarget(file); target != nil {
			BazelLog.Tracef("add '%s' src '%s/%s'", target.name, args.Rel, file)

			groupFiles, _ := sourceFileGroups.Get(target.name)
			groupFiles.(*treeset.Set).Add(file)
		} else {
			BazelLog.Tracef("Skip src '%s'", file)
		}
	}

	// Determine if this project should be exposed as an npm package.
	// If exposed as an npm package make the npm package the primary target.
	isNpmPackage := ts.pnpmProjects.IsProject(args.Rel) && ts.pnpmProjects.IsReferenced(args.Rel)

	// The package/directory name variable value used to render the target names.
	packageName := gazelle.ToDefaultTargetName(args, DefaultRootTargetName)

	// Create rules for each target group.
	sourceRules := treemap.NewWithStringComparator()
	for _, group := range cfg.GetSourceTargets() {
		// The project rule name. Can be configured to map to a different name.
		ruleName := cfg.RenderSourceTargetName(group.name, packageName, isNpmPackage)

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
				args,
				ruleName,
				ruleSrcs,
				dataFiles,
				group.testonly,
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
	if isNpmPackage {
		npmPackageName := cfg.RenderNpmPackageTargetName(packageName)
		npmPackageSourceFiles := treeset.NewWithStringComparator()

		if srcRule, _ := sourceRules.Get(DefaultLibraryName); srcRule != nil {
			srcProjectLabel := label.Label{
				Name:     srcRule.(*rule.Rule).Name(),
				Repo:     args.Config.RepoName,
				Pkg:      args.Rel,
				Relative: true,
			}

			// Add the src to the pkg
			npmPackageSourceFiles.Add(srcProjectLabel.String())
		}

		// Add the package.json if not in the src
		if dataFiles.Contains(NpmPackageFilename) {
			dataFiles.Remove(NpmPackageFilename)
			npmPackageSourceFiles.Add(NpmPackageFilename)
		}

		ts.addNpmPackageRule(
			cfg,
			args,
			npmPackageName,
			npmPackageSourceFiles,
			result,
		)
	}
}

func (ts *typeScriptLang) addNpmPackageRule(cfg *JsGazelleConfig, args language.GenerateArgs, npmPackageName string, srcs *treeset.Set, result *language.GenerateResult) {
	npmPackage := rule.NewRule(NpmPackageKind, npmPackageName)
	npmPackage.SetAttr("srcs", srcs.Values())

	result.Gen = append(result.Gen, npmPackage)
	result.Imports = append(result.Imports, newNpmPackageImports())

	BazelLog.Infof("add rule '%s' '%s:%s'", npmPackage.Kind(), args.Rel, npmPackage.Name())
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
		imports = append(imports, ImportStatement{
			ImportSpec: resolve.ImportSpec{
				Lang: LanguageName,
				Imp:  tsconfig.Extends,
			},
			ImportPath: tsconfig.Extends,
			SourcePath: SourcePath,
		})
	}

	for _, t := range tsconfig.Types {
		imports = append(imports, ImportStatement{
			ImportSpec: resolve.ImportSpec{
				Lang: LanguageName,
				Imp:  toAtTypesPackage(t),
			},
			ImportPath: t,
			SourcePath: SourcePath,
		})
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

func (ts *typeScriptLang) addProjectRule(cfg *JsGazelleConfig, args language.GenerateArgs, targetName string, sourceFiles, dataFiles *treeset.Set, isTestRule bool, result *language.GenerateResult) (*rule.Rule, error) {
	// Check for name-collisions with the rule being generated.
	colError := gazelle.CheckCollisionErrors(targetName, TsProjectKind, sourceRuleKinds, args)
	if colError != nil {
		return nil, fmt.Errorf(colError.Error()+" "+
			"Use the '# gazelle:%s' directive to change the naming convention.\n\n"+
			"For example:\n"+
			"\t# gazelle:%s {dirname}_js\n"+
			"\t# gazelle:%s {dirname}_js_tests",
			Directive_LibraryNamingConvention,
			Directive_LibraryNamingConvention,
			Directive_TestsNamingConvention,
		)
	}

	// Project data combined from all files.
	info := newTsProjectInfo()

	for result := range ts.parseFiles(cfg, args, sourceFiles) {
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
			sourceFiles.Add(dataFile)
			dataFiles.Remove(dataFile)
		}
	}

	ruleKind := TsProjectKind
	if !hasTranspiledSources(sourceFiles) {
		ruleKind = JsLibraryKind
	}
	sourceRule := rule.NewRule(ruleKind, targetName)
	sourceRule.SetPrivateAttr("ts_project_info", info)
	sourceRule.SetAttr("srcs", sourceFiles.Values())

	if isTestRule {
		sourceRule.SetAttr("testonly", true)
	}

	if cfg.GetTsConfigGenerationEnabled() {
		if rel, tsconfig := ts.tsconfig.ResolveConfig(args.Rel); tsconfig != nil {
			tsconfigLabel := label.New("", rel, cfg.RenderTsConfigName(tsconfig.ConfigName))
			tsconfigLabel = tsconfigLabel.Rel("", args.Rel)
			sourceRule.SetAttr("tsconfig", tsconfigLabel.String())
		}
	}

	adaptExistingRule(args, sourceRule)

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
					Alt:        []string{},
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
				resultsChannel <- ts.collectImports(cfg, args.Config.RepoRoot, sourcePath)
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

func (ts *typeScriptLang) collectImports(cfg *JsGazelleConfig, rootDir, sourcePath string) parseResult {
	parseResults, errs := parseSourceFile(rootDir, sourcePath)

	result := parseResult{
		SourcePath: sourcePath,
		Errors:     errs,
		Imports:    make([]ImportStatement, 0, len(parseResults.Imports)),
		Modules:    parseResults.Modules,
	}

	for _, importPath := range parseResults.Imports {
		// The path from the root
		workspacePath := toWorkspacePath(sourcePath, importPath)

		if !cfg.IsImportIgnored(importPath) {
			alternates := make([]string, 0)
			for _, alt := range ts.tsconfig.ExpandPaths(sourcePath, importPath) {
				alternates = append(alternates, toWorkspacePath(sourcePath, alt))
			}

			// Record all imports. Maybe local, maybe data, maybe in other BUILD etc.
			result.Imports = append(result.Imports, ImportStatement{
				ImportSpec: resolve.ImportSpec{
					Lang: LanguageName,
					Imp:  workspacePath,
				},
				Alt:        alternates,
				ImportPath: importPath,
				SourcePath: sourcePath,
			})

			BazelLog.Tracef("Import: %q -> %q (alias: %v)", workspacePath, importPath, alternates)
		} else {
			BazelLog.Tracef("Import ignored: %q -> %q", workspacePath, importPath)
		}
	}

	return result
}

// Parse the passed file for import statements.
func parseSourceFile(rootDir, filePath string) (parser.ParseResult, []error) {
	BazelLog.Debugf("ParseImports: %s", filePath)

	content, err := os.ReadFile(path.Join(rootDir, filePath))
	if err != nil {
		return parser.ParseResult{}, []error{err}
	}

	p := treesitter_parser.NewParser()
	return p.ParseSource(filePath, string(content))
}

func (ts *typeScriptLang) collectSourceFiles(cfg *JsGazelleConfig, args language.GenerateArgs) (*treeset.Set, *treeset.Set, error) {
	sourceFiles := treeset.NewWithStringComparator()
	dataFiles := treeset.NewWithStringComparator()

	// Do not recurse into sub-directories if generating a BUILD per directory
	recurse := cfg.GenerationMode() != GenerationModeDirectory

	err := gazelle.GazelleWalkDir(args, ts.gitignore, cfg.excludes, recurse, func(f string) error {
		// Excluded due to being outside the ts root
		if !ts.tsconfig.IsWithinTsRoot(f) {
			BazelLog.Debugf("Skip %q outside rootDir\n", f)
			return filepath.SkipDir
		}

		// Otherwise the file is either source or potentially importable.
		if isSourceFileType(f) {
			sourceFiles.Add(f)
		} else if isDataFileType(f) {
			dataFiles.Add(f)
		}

		return nil
	})

	return sourceFiles, dataFiles, err
}

func (ts *typeScriptLang) addFileLabel(importPath string, label *label.Label) {
	existing := ts.fileLabels[importPath]

	if existing != nil {
		// Can not have two imports (such as .js and .d.ts) from different labels
		if isDeclarationFileType(existing.Name) == isDeclarationFileType(label.Name) && !existing.Equal(*label) {
			BazelLog.Fatalf("Duplicate file label ", importPath, " from ", existing.String(), " and ", label.String())
		}

		// Prefer the non-declaration file
		if isDeclarationFileType(existing.Name) {
			return
		}

		// Otherwise overwrite the existing non-declaration version
	}

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
	paths := make([]string, 0, 1)

	if isDeclarationFileType(p) {
		// With the js extension
		paths = append(paths, swapDeclarationExtension(p))

		// Without the js extension
		if isImplicitSourceFileType(p) {
			paths = append(paths, stripDeclarationExtensions(p))
		}

		// Directory without the filename
		if path.Base(stripDeclarationExtensions(p)) == IndexFileName {
			paths = append(paths, path.Dir(p))
		}
	} else if isSourceFileType(p) {
		// With the transpiled .js extension
		if isTranspiledSourceFileType(p) {
			// With the js extension
			paths = append(paths, swapSourceExtension(p))
		}

		// Without the js extension
		if isImplicitSourceFileType(p) {
			paths = append(paths, stripSourceFileExtension(p))
		}

		// Directory without the filename
		if path.Base(stripSourceFileExtension(p)) == IndexFileName {
			paths = append(paths, path.Dir(p))
		}
	} else if isDataFileType(p) {
		paths = append(paths, p)
	}

	return paths
}

// Collect and persist all possible references to files that can be imported
func (ts *typeScriptLang) collectFileLabels(args language.GenerateArgs) map[string]*label.Label {
	generators := make(map[string]*label.Label)

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

	return generators
}

// Add rules representing packages, node_modules etc
func (ts *typeScriptLang) addPackageRules(cfg *JsGazelleConfig, args language.GenerateArgs, result *language.GenerateResult) {
	if ts.pnpmProjects.IsProject(args.Rel) {
		addLinkAllPackagesRule(cfg, args, result)
	}
}

// Add pnpm rules for a pnpm lockfile.
// Collect pnpm projects and project dependencies from the lockfile.
func (ts *typeScriptLang) addPnpmLockfile(cfg *JsGazelleConfig, repoName, repoRoot, lockfile string) {
	BazelLog.Infof("add workspace %q", lockfile)

	lockfilePath := path.Join(repoRoot, lockfile)

	pnpmWorkspace := ts.pnpmProjects.NewWorkspace(lockfile)
	pnpmRefs := make(map[string]string)

	for project, packages := range pnpm.ParsePnpmLockFileDependencies(lockfilePath) {
		BazelLog.Debugf("add %q project %q ", lockfile, project)

		pnpmProject := pnpmWorkspace.AddProject(project)

		for pkg, version := range packages {
			BazelLog.Tracef("add %q npm package: %q", project, pkg)

			pnpmProject.AddPackage(pkg, &label.Label{
				Repo:     repoName,
				Pkg:      pnpmProject.Pkg(),
				Name:     path.Join(cfg.npmLinkAllTargetName, pkg),
				Relative: false,
			})

			// If this is a local workspace link or file reference normalize the path and collect the references
			if strings.Index(version, "link:") == 0 {
				link := version[len("link:"):]

				BazelLog.Tracef("add %q project reference to project %q as %q", project, link, pkg)

				// Pnpm "link" references are relative to the package defining the link
				pnpmRefs[pkg] = path.Join(pnpmProject.Pkg(), link)
			} else if strings.Index(version, "file:") == 0 {
				file := version[len("file:"):]

				BazelLog.Tracef("add %q project reference to file %q as %q", project, file, pkg)

				// Pnpm "file" references are relative to the pnpm workspace root.
				pnpmRefs[pkg] = path.Join(path.Dir(lockfile), file)
			}
		}
	}

	// Record the collected references
	for pkg, ref := range pnpmRefs {
		pnpmWorkspace.AddReference(pkg, ref)
	}
}

func addLinkAllPackagesRule(cfg *JsGazelleConfig, args language.GenerateArgs, result *language.GenerateResult) {
	npmLinkAll := rule.NewRule(NpmLinkAllKind, cfg.npmLinkAllTargetName)

	result.Gen = append(result.Gen, npmLinkAll)
	result.Imports = append(result.Imports, newLinkAllPackagesImports())

	BazelLog.Infof("add rule '%s' '%s:%s'", npmLinkAll.Kind(), args.Rel, npmLinkAll.Name())
}

// Adapted an existing rule to a new rule of the same name.
func adaptExistingRule(args language.GenerateArgs, rule *rule.Rule) {
	existing := gazelle.GetFileRuleByName(args, rule.Name())
	if existing == nil {
		return
	}

	// TODO: this seems like a hack...
	// Gazelle should support new rules changing the type of existing rules?
	if existing.Kind() != rule.Kind() {
		existing.SetKind(rule.Kind())
	}
}

// If the file is ts-compatible transpiled source code that may contain imports
func isTranspiledSourceFileType(f string) bool {
	ext := path.Ext(f)
	return len(ext) > 0 && typescriptFileExtensions.Contains(ext[1:]) && !isDeclarationFileType(f)
}

// If the file is ts-compatible source code that may contain imports
func isSourceFileType(f string) bool {
	if isTranspiledSourceFileType(f) || isDeclarationFileType(f) {
		return true
	}

	ext := path.Ext(f)
	return len(ext) > 0 && javascriptFileExtensions.Contains(ext[1:])
}

// A source file that does not explicitly declare itself as cjs or mjs so
// it can be imported as if it is either. Node will decide how to interpret
// it at runtime based on other factors.
func isImplicitSourceFileType(f string) bool {
	return path.Ext(f) == ".ts" || path.Ext(f) == ".tsx" || path.Ext(f) == ".js" || path.Ext(f) == ".jsx"
}

func isDeclarationFileType(f string) bool {
	for _, ex := range declarationFileExtensionsArray {
		if strings.HasSuffix(f, "."+ex) {
			return true
		}
	}

	return false
}

func isDataFileType(f string) bool {
	ext := path.Ext(f)
	return len(ext) > 0 && dataFileExtensions.Contains(ext[1:])
}

// Strip extensions off of a path, assuming it isSourceFileType()
func stripSourceFileExtension(f string) string {
	return f[:len(f)-len(path.Ext(f))]
}

// Swap compile to runtime extensions of of a path, assuming it isSourceFileType()
func swapSourceExtension(f string) string {
	return stripSourceFileExtension(f) + toJsExt(f)
}

// Strip extensions off of a path, assuming it isDeclarationFileType()
func stripDeclarationExtensions(f string) string {
	return stripSourceFileExtension(stripSourceFileExtension(f))
}

// Swap compile to runtime extensions of of a path, assuming it isDeclarationFileType()
func swapDeclarationExtension(f string) string {
	return stripDeclarationExtensions(f) + toJsExt(f)
}

func toJsExt(f string) string {
	e := path.Ext(f)
	e = strings.Replace(e, "tsx", "js", 1)
	e = strings.Replace(e, "ts", "js", 1)
	return e
}

// Normalize the given import statement from a relative path
// to a path relative to the workspace.
func toWorkspacePath(importFrom, importPath string) string {
	// Convert relative to workspace-relative
	if importPath[0] == '.' {
		importPath = path.Join(path.Dir(importFrom), importPath)
	}

	// Clean any extra . / .. etc
	return path.Clean(importPath)
}
