package gazelle

import (
	"fmt"
	"os"
	"path"
	"strings"
	"time"

	common "aspect.build/cli/gazelle/common"
	starlark "aspect.build/cli/gazelle/common/starlark"
	node "aspect.build/cli/gazelle/js/node"
	proto "aspect.build/cli/gazelle/js/proto"
	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/repo"
	"github.com/bazelbuild/bazel-gazelle/resolve"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"github.com/emirpasic/gods/sets/treeset"
)

// typeScriptLang satisfies the resolve.Resolver interface. It resolves dependencies
// in rules generated by this extension.
var _ resolve.Resolver = (*typeScriptLang)(nil)

const (
	Resolution_Error      = -1
	Resolution_None       = 0
	Resolution_NotFound   = 1
	Resolution_Package    = 2
	Resolution_Label      = 3
	Resolution_NativeNode = 4
	Resolution_Override   = 5
)

type ResolutionType = int

// Name returns the name of the language. This is the prefix of the kinds of
// rules generated. E.g. ts_project
func (*typeScriptLang) Name() string { return LanguageName }

// Determine what rule (r) outputs which can be imported.
// For TypeScript this is all the import-paths pointing to files within the rule.
func (ts *typeScriptLang) Imports(c *config.Config, r *rule.Rule, f *rule.File) []resolve.ImportSpec {
	BazelLog.Debugf("Imports(%s): //%s:%s", LanguageName, f.Pkg, r.Name())

	switch r.Kind() {
	case TsProtoLibraryKind:
		return ts.protoLibraryImports(r, f)
	case TsConfigKind:
		return ts.tsconfigImports(r, f)
	case TsProjectKind:
		fallthrough
	case JsLibraryKind:
		return ts.sourceFileImports(c, r, f)
	}
	return nil
}

// TypeScript-importable ImportSpecs from a set of source files.
func (ts *typeScriptLang) sourceFileImports(c *config.Config, r *rule.Rule, f *rule.File) []resolve.ImportSpec {
	var srcs []string

	infoAttr := r.PrivateAttr("ts_project_info")
	if infoAttr != nil && infoAttr.(*TsProjectInfo).sources != nil {
		srcsSet := infoAttr.(*TsProjectInfo).sources
		srcs = make([]string, 0, srcsSet.Size())
		for _, s := range srcsSet.Values() {
			srcs = append(srcs, s.(string))
		}
	} else {
		expandedSrcs, err := starlark.ExpandSrcs(c.RepoRoot, f.Pkg, r.Attr("srcs"))
		if err != nil {
			BazelLog.Debugf("Failed to expand srcs of %s:%s - %v", f.Pkg, r.Name(), err)
			return []resolve.ImportSpec{}
		}

		srcs = expandedSrcs
	}

	_, tsconfig := ts.tsconfig.FindConfig(f.Pkg)

	provides := make([]resolve.ImportSpec, 0, len(srcs)+1)

	// Sources that produce importable paths.
	for _, src := range srcs {
		// The raw source path
		srcs = []string{path.Join(f.Pkg, src)}

		// Also add tsconfig-mapped directories for references
		// to the output files of the ts_project rule.
		if tsconfig != nil {
			outSrc := tsconfig.ToOutDir(src)
			if outSrc != src {
				srcs = append(srcs, path.Join(f.Pkg, outSrc))
			}
		}

		for _, src := range srcs {
			for _, impt := range toImportPaths(src) {
				provides = append(provides, resolve.ImportSpec{
					Lang: LanguageName,
					Imp:  impt,
				})
			}
		}
	}

	if len(provides) == 0 {
		return nil
	}

	return provides
}

func (ts *typeScriptLang) tsconfigImports(r *rule.Rule, f *rule.File) []resolve.ImportSpec {
	// Only the tsconfig file itself is exposed.
	// The output is the same as the ts_config(src) input.
	return []resolve.ImportSpec{
		{
			Lang: LanguageName,
			Imp:  path.Join(f.Pkg, r.AttrString("src")),
		},
	}
}

// TypeScript-importable ImportSpecs from a TsProtoLibrary rule.
func (ts *typeScriptLang) protoLibraryImports(r *rule.Rule, f *rule.File) []resolve.ImportSpec {
	protoSrcsAttr := r.PrivateAttr("proto_library_srcs")

	// The rule may not have been generated by this gazelle plugin
	if protoSrcsAttr == nil {
		return nil
	}

	protoSrcs := protoSrcsAttr.([]string)
	provides := make([]resolve.ImportSpec, 0, len(protoSrcs)+1)

	for _, src := range protoSrcs {
		src = path.Join(f.Pkg, src)

		for _, dts := range proto.ToTsPaths(src) {
			for _, impt := range toImportPaths(dts) {
				provides = append(provides, resolve.ImportSpec{
					Lang: LanguageName,
					Imp:  impt,
				})
			}
		}
	}

	if len(provides) == 0 {
		return nil
	}

	return provides
}

// Embeds returns a list of labels of rules that the given rule embeds. If
// a rule is embedded by another importable rule of the same language, only
// the embedding rule will be indexed. The embedding rule will inherit
// the imports of the embedded rule.
func (ts *typeScriptLang) Embeds(r *rule.Rule, from label.Label) []label.Label {
	BazelLog.Debugf("Embeds(%s): '//%s:%s'", LanguageName, from.Pkg, r.Name())

	switch r.Kind() {
	case TsProjectKind:
		srcs := r.AttrStrings("srcs")
		tsEmbeds := make([]label.Label, 0, len(srcs))

		// The compiled dts and js files are accessible as embedded rules
		for _, src := range srcs {
			if isTranspiledSourceFileType(src) {
				pExt := path.Ext(src)
				pNoExt := src[:len(src)-len(pExt)]
				js := pNoExt + toJsExt(pExt)
				dts := pNoExt + toDtsExt(pExt)

				tsEmbeds = append(tsEmbeds, label.New(from.Repo, from.Pkg, js))
				tsEmbeds = append(tsEmbeds, label.New(from.Repo, from.Pkg, dts))
			}
		}

		return tsEmbeds
	}

	// TODO(jbedard): ts_proto_library() embeds

	// TODO(jbedard): implement other rule kinds
	return make([]label.Label, 0)
}

// Resolve translates imported libraries for a given rule into Bazel
// dependencies. Information about imported libraries is returned for each
// rule generated by language.GenerateRules in
// language.GenerateResult.Imports. Resolve generates a "deps" attribute (or
// the appropriate language-specific equivalent) for each import according to
// language-specific rules and heuristics.
func (ts *typeScriptLang) Resolve(
	c *config.Config,
	ix *resolve.RuleIndex,
	rc *repo.RemoteCache,
	r *rule.Rule,
	importData interface{},
	from label.Label,
) {
	start := time.Now()
	BazelLog.Infof("Resolve(%s): //%s:%s", LanguageName, from.Pkg, r.Name())

	// TsProject imports are resolved as deps
	switch r.Kind() {
	case TsProjectKind, JsLibraryKind, TsConfigKind, TsProtoLibraryKind:
		deps := common.NewLabelSet(from)

		// Support this target representing a project or a package
		var imports *treeset.Set
		if packageInfo, isPackageInfo := importData.(*TsPackageInfo); isPackageInfo {
			imports = packageInfo.imports

			if packageInfo.source != nil {
				deps.Add(packageInfo.source)
			}
		} else if projectInfo, isProjectInfo := importData.(*TsProjectInfo); isProjectInfo {
			imports = projectInfo.imports
		} else {
			BazelLog.Infof("%s //%s:%s with no/unknown package info", r.Kind(), from.Pkg, r.Name())
			break
		}

		err := ts.resolveImports(c, ix, deps, imports, from)
		if err != nil {
			BazelLog.Fatalf("Resolution Error: %v", err)
			os.Exit(1)
		}

		if r.Kind() == TsProjectKind {
			ts.addTsLib(c, ix, deps, from)
		}

		if !deps.Empty() {
			r.SetAttr("deps", deps.Labels())
		}
	case NpmPackageKind:
		packageInfo, isPackageInfo := importData.(*TsPackageInfo)
		if !isPackageInfo {
			BazelLog.Infof("%s //%s:%s with no/unknown package info", r.Kind(), from.Pkg, r.Name())
			break
		}

		srcs := packageInfo.sources.Values()

		deps := common.NewLabelSet(from)
		err := ts.resolveImports(c, ix, deps, packageInfo.imports, from)
		if err != nil {
			BazelLog.Fatalf("Resolution Error: %v", err)
			os.Exit(1)
		}
		for _, dep := range deps.Labels() {
			srcs = append(srcs, dep.String())
		}

		if packageInfo.source != nil {
			srcs = append(srcs, packageInfo.source.String())
		}

		if len(srcs) > 0 {
			r.SetAttr("srcs", srcs)
		}
	}

	BazelLog.Infof("Resolve(%s): //%s:%s DONE in %s", LanguageName, from.Pkg, r.Name(), time.Since(start).String())
}
func (ts *typeScriptLang) addTsLib(
	c *config.Config,
	ix *resolve.RuleIndex,
	deps *common.LabelSet,
	from label.Label,
) {
	_, tsconfig := ts.tsconfig.FindConfig(from.Pkg)
	if tsconfig != nil && tsconfig.ImportHelpers {
		if tslibLabel := ts.resolvePackage(from, "tslib"); tslibLabel != nil {
			deps.Add(tslibLabel)
		}
	}
}

func (ts *typeScriptLang) resolveImports(
	c *config.Config,
	ix *resolve.RuleIndex,
	deps *common.LabelSet,
	imports *treeset.Set,
	from label.Label,
) error {
	cfg := c.Exts[LanguageName].(*JsGazelleConfig)

	resolutionErrors := []error{}

	it := imports.Iterator()
	for it.Next() {
		imp := it.Value().(ImportStatement)

		resolutionType, dep, err := ts.resolveImport(c, ix, from, imp)
		if err != nil {
			return err
		}

		types := ts.resolveImportTypes(resolutionType, from, imp)
		for _, typesDep := range types {
			deps.Add(typesDep)
		}

		if dep != nil {
			deps.Add(dep)
		}

		// Neither the import or a type definition was found.
		if resolutionType == Resolution_NotFound && len(types) == 0 {
			if imp.Optional {
				BazelLog.Infof("Optional import %q for target %q not found", imp.ImportPath, from.String())
			} else if cfg.ValidateImportStatements() != ValidationOff {
				BazelLog.Debugf("import %q for target %q not found", imp.ImportPath, from.String())

				notFound := fmt.Errorf(
					"Import %[1]q from %[2]q is an unknown dependency. Possible solutions:\n"+
						"\t1. Instruct Gazelle to resolve to a known dependency using a directive:\n"+
						"\t\t# aspect:resolve [src-lang] js import-string label\n"+
						"\t\t   or\n"+
						"\t\t# aspect:js_resolve import-string-glob label\n"+
						"\t2. Ignore the dependency using the '# aspect:%[3]s %[1]s' directive.\n"+
						"\t3. Disable Gazelle resolution validation using '# aspect:%[4]s off'",
					imp.ImportPath, imp.SourcePath, Directive_IgnoreImports, Directive_ValidateImportStatements,
				)
				resolutionErrors = append(resolutionErrors, notFound)
			}

			continue
		}
	}

	// Log any resolution errorsResolution errors and error out.
	if len(resolutionErrors) > 0 {
		joinedErrs := ""
		for _, err := range resolutionErrors {
			joinedErrs = fmt.Sprintf("%s\n\n%s", joinedErrs, err)
		}

		switch cfg.ValidateImportStatements() {
		case ValidationError:
			fmt.Fprintf(os.Stderr, "Failed to validate dependencies for target %q:%v\n", from.String(), joinedErrs)
			os.Exit(1)
		case ValidationWarn:
			fmt.Fprintf(os.Stderr, "Warning: Failed to validate dependencies for target %q:%v\n", from.String(), joinedErrs)
		}
	}

	return nil
}

func (ts *typeScriptLang) resolveImport(
	c *config.Config,
	ix *resolve.RuleIndex,
	from label.Label,
	impStm ImportStatement,
) (ResolutionType, *label.Label, error) {
	cfg := c.Exts[LanguageName].(*JsGazelleConfig)

	imp := impStm.ImportSpec

	// Overrides
	if override, ok := resolve.FindRuleWithOverride(c, imp, LanguageName); ok {
		return Resolution_Override, &override, nil
	}

	// JS Overrides (js_resolve)
	if res := cfg.GetResolution(imp.Imp); res != nil {
		return Resolution_Override, res, nil
	}

	// Gazelle rule index
	if resolution, match, err := ts.resolveImportFromIndex(c, ix, from, impStm); resolution != Resolution_NotFound {
		return resolution, match, err
	}

	// References to a label such as a file or file-generating rule
	if importLabel := ts.getImportLabel(imp.Imp); importLabel != nil {
		return Resolution_Label, importLabel, nil
	}

	// References to an npm package, pnpm workspace projects etc.
	if pkg := ts.resolvePackageImport(from, impStm.Imp); pkg != nil {
		return Resolution_Package, pkg, nil
	}

	// References via tsconfig mappings (paths, baseUrl, rootDirs etc.)
	if tsconfigPaths := ts.tsconfig.ExpandPaths(impStm.SourcePath, impStm.ImportPath); len(tsconfigPaths) > 0 {
		for _, p := range tsconfigPaths {
			pImp := ImportStatement{
				ImportSpec: resolve.ImportSpec{
					Lang: impStm.ImportSpec.Lang,
					Imp:  toImportSpecPath(impStm.SourcePath, p),
				},
				SourcePath: impStm.SourcePath,
				ImportPath: impStm.ImportPath,
				Optional:   impStm.Optional,
			}
			if resolution, match, err := ts.resolveImportFromIndex(c, ix, from, pImp); resolution != Resolution_NotFound {
				return resolution, match, err
			}
		}
	}

	// Native node imports
	if node.IsNodeImport(imp.Imp) {
		return Resolution_NativeNode, nil, nil
	}

	return Resolution_NotFound, nil, nil
}

func (ts *typeScriptLang) resolveImportFromIndex(
	c *config.Config,
	ix *resolve.RuleIndex,
	from label.Label,
	impStm ImportStatement) (ResolutionType, *label.Label, error) {

	matches := ix.FindRulesByImportWithConfig(c, impStm.ImportSpec, LanguageName)
	if len(matches) == 0 {
		return Resolution_NotFound, nil, nil
	}

	filteredMatches := make([]label.Label, 0, len(matches))
	for _, match := range matches {
		// Prevent from adding itself as a dependency.
		if !match.IsSelfImport(from) {
			filteredMatches = append(filteredMatches, match.Label)
		}
	}

	// Too many results, don't know which is correct
	if len(filteredMatches) > 1 {
		return Resolution_Error, nil, fmt.Errorf(
			"Import %q from %q resolved to multiple targets (%s) - this must be fixed using the \"aspect:resolve\" directive",
			impStm.ImportPath, impStm.SourcePath, targetListFromResults(matches))
	}

	// The matches were self imports, no dependency is needed
	if len(filteredMatches) == 0 {
		return Resolution_None, nil, nil
	}

	match := filteredMatches[0]

	BazelLog.Tracef("resolve %q import %q as %q", from, impStm.Imp, match)

	return Resolution_Override, &match, nil
}

func (ts *typeScriptLang) resolvePackageImport(from label.Label, imp string) *label.Label {
	impPkg, _ := node.ParseImportPath(imp)

	// Imports not in the form of a package
	if impPkg == "" {
		return nil
	}

	return ts.resolvePackage(from, impPkg)
}

func (ts *typeScriptLang) resolvePackage(from label.Label, impPkg string) *label.Label {
	fromProject := ts.pnpmProjects.GetProject(from.Pkg)
	if fromProject == nil {
		BazelLog.Tracef("resolve %q import %q project not found", from.String(), impPkg)
		return nil
	}

	impPkgLabel := fromProject.Get(impPkg)
	if impPkgLabel == nil {
		BazelLog.Tracef("resolve %q import %q not found", from.String(), impPkg)
		return nil
	}

	BazelLog.Tracef("resolve %q import %q to %q", from.String(), impPkg, impPkgLabel)

	return impPkgLabel
}

func (ts *typeScriptLang) resolveImportTypes(resolutionType ResolutionType, from label.Label, imp ImportStatement) []*label.Label {
	// Overrides are not extended with additional types
	if resolutionType == Resolution_Override {
		return nil
	}

	// Types for native node imports are always resolved to @types/node
	if resolutionType == Resolution_NativeNode {
		if typesNode := ts.resolveAtTypes(from, "node"); typesNode != nil {
			return []*label.Label{typesNode}
		}

		return nil
	}

	// Packages with specific @types/* definitions
	if typesPkg := ts.resolveAtTypes(from, imp.Imp); typesPkg != nil {
		// @types packages for any named imports
		// The import may be a package, may be an unresolved import with only @types
		return []*label.Label{typesPkg}
	}

	// If an import has not been found and has no designated package or @types package
	// then fallback to any custom module definitions such as 'declare module' statements.
	if resolutionType == Resolution_NotFound {
		// Custom module definitions for the import if there is no other resolution
		if typeModules := ts.moduleTypes[imp.Imp]; typeModules != nil {
			return typeModules
		}
	}

	// No types found
	return nil
}

func toAtTypesPackage(pkg string) string {
	slashI := strings.Index(pkg, "/")

	// Change the scoped packages to be __ separated.
	if pkg[0] == '@' && slashI != -1 {
		pkg = pkg[1:slashI] + "__" + pkg[slashI+1:]
		slashI = strings.Index(pkg, "/")
	}

	// Strip any trailing subpaths
	if slashI != -1 {
		pkg = pkg[:slashI]
	}

	return "@types/" + pkg
}

// Find and resolve any @types package for an import
func (ts *typeScriptLang) resolveAtTypes(from label.Label, imp string) *label.Label {
	fromProject := ts.pnpmProjects.GetProject(from.Pkg)
	if fromProject == nil {
		return nil
	}

	typesPkg := toAtTypesPackage(imp)

	return fromProject.Get(typesPkg)
}

// targetListFromResults returns a string with the human-readable list of
// targets contained in the given results.
func targetListFromResults(results []resolve.FindResult) string {
	list := make([]string, len(results))
	for i, result := range results {
		list[i] = result.Label.String()
	}
	return strings.Join(list, ", ")
}
