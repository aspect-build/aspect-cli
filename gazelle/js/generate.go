package gazelle

import (
	"fmt"
	"log"
	"os"
	"path"
	"strings"

	pnpm "aspect.build/cli/gazelle/js/pnpm"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/bazelbuild/bazel-gazelle/resolve"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"github.com/emirpasic/gods/maps/treemap"
	"github.com/emirpasic/gods/sets/treeset"
)

type Language struct {
	language.Language
}

const (
	// The filename (with any of the TS extensions) imported when importing a directory.
	IndexFileName = "index"

	NpmPackageFilename = "package.json"

	DefaultRootTargetName = "root"
)

func (ts *TypeScript) GetImportLabel(imp string) *label.Label {
	return ts.fileLabels[imp]
}

// GenerateRules extracts build metadata from source files in a directory.
// GenerateRules is called in each directory where an update is requested
// in depth-first post-order.
func (ts *TypeScript) GenerateRules(args language.GenerateArgs) language.GenerateResult {
	cfgs := args.Config.Exts[LanguageName].(Configs)
	cfg := cfgs.Get(args.Rel)

	// Collect any labels that could be imported
	ts.collectFileLabels(args)

	// When we return empty, we mean that we don't generate anything, but this
	// still triggers the indexing for all the TypeScript targets in this package.
	if !cfg.GenerationEnabled() {
		BazelLog.Tracef("GenerateRules disabled %s", args.Rel)
		return language.GenerateResult{}
	}

	// If this directory has not been declared as a bazel package only continue
	// if generating new BUILD files is enabled.
	if cfg.GenerationMode() == GenerationModeNone && !isBazelPackage(args.Dir) {
		return language.GenerateResult{}
	}

	BazelLog.Tracef("GenerateRules '%s'", args.Rel)

	var result language.GenerateResult

	ts.addPackageRules(cfg, args, &result)
	ts.addSourceRules(cfg, args, &result)

	return result
}

func (ts *TypeScript) addSourceRules(cfg *JsGazelleConfig, args language.GenerateArgs, result *language.GenerateResult) {
	// Collect all source files.
	sourceFiles, dataFiles, collectErr := collectSourceFiles(cfg, args)
	if collectErr != nil {
		log.Printf("Source collection error: %v\n", collectErr)
		return
	}

	// Divide src vs test files.
	sourceFileGroups := treemap.NewWithStringComparator()
	for _, group := range cfg.GetSourceGroups() {
		sourceFileGroups.Put(group, treeset.NewWithStringComparator())
	}

	for _, f := range sourceFiles.Values() {
		file := f.(string)
		if cfg.IsTestFile(file) {
			BazelLog.Tracef("Add test src '%s'", file)

			group, _ := sourceFileGroups.Get(DefaultTestsName)
			group.(*treeset.Set).Add(file)
		} else if cfg.IsSourceFile(file) {
			BazelLog.Tracef("Add src '%s'", file)

			group, _ := sourceFileGroups.Get(DefaultLibraryName)
			group.(*treeset.Set).Add(file)
		} else {
			BazelLog.Tracef("Skip src '%s'", file)
		}
	}

	// Determine if this project should be exposed as an npm package.
	// If exposed as an npm package make the npm package the primary target.
	isNpmPackage := ts.pnpmProjects.IsProject(args.Rel) && ts.pnpmProjects.IsReferenced(args.Rel)

	// Make the package rule the default target for this BUILD.
	packageName := toTargetPackageName(args)

	// Build the GenerateResult with src and test rules.
	srcRuleSrcs, _ := sourceFileGroups.Get(DefaultLibraryName)
	srcRule, srcGenErr := addProjectRule(
		cfg,
		args,
		cfg.RenderTargetName(cfg.libraryNamingConvention, packageName),
		srcRuleSrcs.(*treeset.Set),
		dataFiles,
		false,
		isNpmPackage,
		result,
	)
	if srcGenErr != nil {
		log.Printf("Source rule generation error: %v\n", srcGenErr)
		os.Exit(1)
	}

	testRuleSrcs, _ := sourceFileGroups.Get(DefaultTestsName)
	_, testGenErr := addProjectRule(
		cfg,
		args,
		cfg.RenderTargetName(cfg.testsNamingConvention, packageName),
		testRuleSrcs.(*treeset.Set),
		dataFiles,
		true,
		false,
		result,
	)
	if testGenErr != nil {
		log.Printf("Test rule generation error: %v\n", testGenErr)
		os.Exit(1)
	}

	// If this is a package wrap the main ts_project() rule with npm_package()
	if isNpmPackage {
		npmPackageName := cfg.RenderTargetName(cfg.npmPackageNamingConvention, packageName)
		npmPackageSourceFiles := treeset.NewWithStringComparator()

		if srcRule != nil {
			srcProjectLabel := label.Label{
				Name:     srcRule.Name(),
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

		addNpmPackageRule(
			cfg,
			args,
			npmPackageName,
			npmPackageSourceFiles,
			result,
		)
	}
}

func addNpmPackageRule(cfg *JsGazelleConfig, args language.GenerateArgs, npmPackageName string, srcs *treeset.Set, result *language.GenerateResult) {
	npmPackage := rule.NewRule(NpmPackageKind, npmPackageName)
	npmPackage.SetAttr("srcs", srcs.Values())

	result.Gen = append(result.Gen, npmPackage)
	result.Imports = append(result.Imports, newNpmPackageImports())

	BazelLog.Infof("add rule '%s' '%s:%s'", npmPackage.Kind(), args.Rel, npmPackage.Name())
}

func addProjectRule(cfg *JsGazelleConfig, args language.GenerateArgs, targetName string, sourceFiles, dataFiles *treeset.Set, isTestRule, isNpmPackage bool, result *language.GenerateResult) (*rule.Rule, error) {
	if isNpmPackage {
		targetName = cfg.RenderNpmSourceLibraryName(targetName)
	}

	// Generate nothing if there are no source files. Remove any existing rules.
	if sourceFiles.Empty() {
		if args.File == nil {
			return nil, nil
		}

		for _, r := range args.File.Rules {
			if r.Name() == targetName && r.Kind() == TsProjectKind {
				emptyRule := rule.NewRule(TsProjectKind, targetName)
				result.Empty = append(result.Empty, emptyRule)
				return emptyRule, nil
			}
		}

		return nil, nil
	}

	// If a build already exists check for name-collisions with the rule being generated.
	if args.File != nil {
		colError := checkCollisionErrors(targetName, args)
		if colError != nil {
			return nil, colError
		}
	}

	// Data file lookup map. Workspace path => local path
	dataFileWorkspacePaths := treemap.NewWithStringComparator()
	for _, dataFile := range dataFiles.Values() {
		dataFileWorkspacePaths.Put(path.Join(args.Rel, dataFile.(string)), dataFile)
	}

	// Data files imported by sourceFiles.
	importedDataFiles := treeset.NewWithStringComparator()

	// Collect import statements from source.
	imports := newTsProjectImports()

	// TODO(jbedard): parse files concurrently
	sourceFileIt := sourceFiles.Iterator()
	for sourceFileIt.Next() {
		filePath := sourceFileIt.Value().(string)

		// Don't parse non-source files such as json
		if !isSourceFileType(filePath) {
			continue
		}

		fileImports, errs := parseImports(args.Config.RepoRoot, path.Join(args.Rel, filePath))

		if len(errs) > 0 {
			fmt.Println(path.Join(args.Rel, filePath), "parse error(s):")
			for _, err := range errs {
				fmt.Println("    ", err)
			}
		}

		for _, importPath := range fileImports {
			if !cfg.IsImportIgnored(importPath) {
				// The path from the root
				workspacePath := toWorkspacePath(args.Rel, filePath, importPath)

				// If the imported path is a file that can be compiled as ts source
				// then add it to the importedDataFiles to be included in the srcs.
				// Remove it from the dataFiles to signify that it is now a "source" file
				// owned by this target.
				if dataFile, _ := dataFileWorkspacePaths.Get(workspacePath); dataFile != nil {
					importedDataFiles.Add(dataFile)
					dataFiles.Remove(dataFile)
				}

				// Record all imports. Maybe local, maybe data, maybe in other BUILD etc.
				imports.Add(ImportStatement{
					ImportSpec: resolve.ImportSpec{
						Lang: LanguageName,
						Imp:  workspacePath,
					},
					ImportPath: importPath,
					SourcePath: path.Join(args.Rel, filePath),
				})
			}
		}
	}

	// Add any imported data files as sources.
	sourceFiles.Add(importedDataFiles.Values()...)

	tsProject := rule.NewRule(TsProjectKind, targetName)
	tsProject.SetAttr("srcs", sourceFiles.Values())

	if isTestRule {
		tsProject.SetAttr("testonly", true)
	}

	result.Gen = append(result.Gen, tsProject)
	result.Imports = append(result.Imports, imports)

	BazelLog.Infof("add rule '%s' '%s:%s'", tsProject.Kind(), args.Rel, tsProject.Name())

	return tsProject, nil
}

// Parse the passed file for import statements.
func parseImports(rootDir, filePath string) ([]string, []error) {
	content, err := os.ReadFile(path.Join(rootDir, filePath))
	if err != nil {
		return nil, []error{err}
	}

	return NewParser().ParseImports(filePath, string(content))
}

// isBazelPackage determines if the directory is a Bazel package by probing for
// the existence of a known BUILD file name.
func isBazelPackage(dir string) bool {
	for _, buildFilename := range buildFileNames {
		buildPath := path.Join(dir, buildFilename)
		if _, err := os.Stat(buildPath); err == nil {
			return true
		}
	}
	return false
}

func isBuildFile(filename string) bool {
	for _, buildFilename := range buildFileNames {
		if filename == buildFilename {
			return true
		}
	}
	return false
}

func collectSourceFiles(cfg *JsGazelleConfig, args language.GenerateArgs) (*treeset.Set, *treeset.Set, error) {
	sourceFiles := treeset.NewWithStringComparator()
	dataFiles := treeset.NewWithStringComparator()

	// Do not recurse into sub-directories if generating a BUILD per directory
	recurse := cfg.GenerationMode() != GenerationModeDirectory

	err := GazelleWalkDir(args, recurse, func(f string, info os.FileInfo, err error) error {
		// Propagate errors.
		if err != nil {
			return err
		}

		if info.IsDir() {
			return nil
		}

		// Excluded files. Must be done manually for additional cfg exclusions
		// such as git/bazelignore support.
		if cfg.IsFileExcluded(f) {
			BazelLog.Tracef("File excluded: %s / %s", args.Rel, f)

			return nil
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

func (ts *TypeScript) addFileLabel(importPath string, label *label.Label) {
	if existing := ts.fileLabels[importPath]; existing != nil {
		log.Fatalln("Duplicate file label ", importPath, " from ", existing.String(), " and ", label.String())
	}

	ts.fileLabels[importPath] = label
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
		// With the js extension
		paths = append(paths, swapSourceExtension(p))

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
func (ts *TypeScript) collectFileLabels(args language.GenerateArgs) map[string]*label.Label {
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
func (ts *TypeScript) addPackageRules(cfg *JsGazelleConfig, args language.GenerateArgs, result *language.GenerateResult) {
	if ts.pnpmProjects.IsProject(args.Rel) {
		addLinkAllPackagesRule(cfg, args, result)
	}
}

// Add pnpm rules for a pnpm lockfile.
// Collect pnpm projects and project dependencies from the lockfile.
func (ts *TypeScript) addPnpmLockfile(cfg *JsGazelleConfig, repoName, repoRoot, lockfile string) {
	BazelLog.Infof("add workspace '%s'", lockfile)

	lockfilePath := path.Join(repoRoot, lockfile)

	pnpmWorkspace := ts.pnpmProjects.NewWorkspace(lockfile)
	pnpmRefs := make(map[string]string)

	for project, packages := range pnpm.ParsePnpmLockFileDependencies(lockfilePath) {
		BazelLog.Debugf("add project '%s' from '%s'", project, lockfile)

		pnpmProject := pnpmWorkspace.AddProject(project)

		for pkg, version := range packages {
			BazelLog.Tracef("add dependency to '%s': '%s'", project, pkg)

			pnpmProject.AddPackage(pkg, &label.Label{
				Repo:     repoName,
				Pkg:      pnpmProject.Pkg(),
				Name:     path.Join(cfg.npmLinkAllTargetName, pkg),
				Relative: false,
			})

			// If this is a local workspace link or file reference normalize the path and collect the references
			if strings.Index(version, "link:") == 0 {
				link := version[len("link:"):]

				BazelLog.Tracef("add project '%s' reference to project '%s' as '%s'", project, link, pkg)

				// Pnpm "link" references are relative to the package defining the link
				pnpmRefs[pkg] = path.Join(pnpmProject.Pkg(), link)
			} else if strings.Index(version, "file:") == 0 {
				file := version[len("file:"):]

				BazelLog.Tracef("add project '%s' reference to file '%s' as '%s'", project, file, pkg)

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

// Check if a target with the same name we are generating alredy exists,
// and if it is of a different kind from the one we are generating. If
// so, we have to throw an error since Gazelle won't generate it correctly.
func checkCollisionErrors(tsProjectTargetName string, args language.GenerateArgs) error {
	tsProjectKind := TsProjectKind

	tsProjectMappedKind := args.Config.KindMap[TsProjectKind].KindName
	if tsProjectMappedKind != "" {
		tsProjectKind = tsProjectMappedKind
	}

	for _, t := range args.File.Rules {
		if t.Name() == tsProjectTargetName && t.Kind() != tsProjectKind {
			fqTarget := label.New("", args.Rel, tsProjectTargetName)
			return fmt.Errorf("failed to generate target %q of kind %q: "+
				"a target of kind %q with the same name already exists. "+
				"Use the '# gazelle:%s' directive to change the naming convention.",
				fqTarget.String(), tsProjectKind, t.Kind(), Directive_LibraryNamingConvention)
		}
	}

	return nil
}

// If the file is ts-compatible source code that may contain typescript imports
func isSourceFileType(f string) bool {
	ext := path.Ext(f)

	// Currently any source files may be parsed as ts and may contain imports
	return len(ext) > 0 && sourceFileExtensions.Contains(ext[1:])
}

// A source file that does not explicitly declare itself as cjs or mjs so
// it can be imported as if it is either. Node will decide how to interpret
// it at runtime based on other factors.
func isImplicitSourceFileType(f string) bool {
	return path.Ext(f) == ".ts"
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
	return stripSourceFileExtension(f) + strings.Replace(path.Ext(f), "ts", "js", 1)
}

// Strip extensions off of a path, assuming it isDeclarationFileType()
func stripDeclarationExtensions(f string) string {
	return stripSourceFileExtension(stripSourceFileExtension(f))
}

// Swap compile to runtime extensions of of a path, assuming it isDeclarationFileType()
func swapDeclarationExtension(f string) string {
	return stripDeclarationExtensions(f) + strings.Replace(path.Ext(f), "ts", "js", 1)
}

// Normalize the given import statement from a relative path
// to a path relative to the workspace.
func toWorkspacePath(pkg, importFrom, importPath string) string {
	// Convert relative to workspace-relative
	if importPath[0] == '.' {
		importPath = path.Join(pkg, path.Dir(importFrom), importPath)
	}

	// Clean any extra . / .. etc
	return path.Clean(importPath)
}

func toTargetPackageName(args language.GenerateArgs) string {
	// The workspace root may be the version control root and non-deterministic
	if args.Rel == "" {
		if args.Config.RepoName != "" {
			return args.Config.RepoName
		} else {
			return DefaultRootTargetName
		}
	}

	return path.Base(args.Dir)
}
