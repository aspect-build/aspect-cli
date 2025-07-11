package gazelle

import (
	"fmt"
	"os"
	"path"
	"strings"
	"time"

	common "github.com/aspect-build/aspect-cli/gazelle/common"
	starlark "github.com/aspect-build/aspect-cli/gazelle/common/starlark"
	node "github.com/aspect-build/aspect-cli/gazelle/js/node"
	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/repo"
	"github.com/bazelbuild/bazel-gazelle/resolve"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"github.com/bazelbuild/buildtools/build"
	"github.com/emirpasic/gods/sets/treeset"
)

// typeScriptLang satisfies the resolve.Resolver interface. It resolves dependencies
// in rules generated by this extension.
var _ resolve.Resolver = (*typeScriptLang)(nil)

const (
	Resolution_Error = iota
	Resolution_None
	Resolution_NotFound
	Resolution_Label
	Resolution_NativeNode
	Resolution_Override
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
		expandedSrcs, err := starlark.ExpandSrcs(c.RepoRoot, f.Pkg, common.GetSourceRegularFiles(c), r.Attr("srcs"))
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

			declarationOutDir := tsconfig.ToDeclarationOutDir(src)
			if outSrc != declarationOutDir && declarationOutDir != src {
				srcs = append(srcs, path.Join(f.Pkg, declarationOutDir))
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
	dtsOutputs := []string{}

	for _, src := range protoSrcs {
		srcPath := path.Join(f.Pkg, src)
		srcBase := strings.TrimSuffix(srcPath, ".proto")

		// Messages: https://github.com/aspect-build/rules_ts/blob/v3.4.0/ts/private/ts_proto_library.bzl#L71
		dtsOutputs = append(dtsOutputs, srcBase+"_pb")

		// Connect: https://github.com/aspect-build/rules_ts/blob/v3.4.0/ts/private/ts_proto_library.bzl#L72-L73
		if starlark.AttrBool(r, "gen_connect_es") {
			dtsOutputs = append(dtsOutputs, srcBase+"_connect")
		}

		// Query services: https://github.com/aspect-build/rules_ts/blob/v3.4.0/ts/private/ts_proto_library.bzl#L74-L78
		if starlark.AttrBool(r, "gen_connect_query") {
			for _, p := range starlark.AttrMap(r, "gen_connect_query_service_mapping") {
				proto := p.Key.(*build.StringExpr).Value
				protoName := strings.TrimSuffix(proto, ".proto")

				if services, isServicesArray := p.Value.(*build.ListExpr); isServicesArray {
					for _, service := range services.List {
						// Service filename: https://github.com/aspect-build/rules_ts/blob/v3.4.0/ts/private/ts_proto_library.bzl#L78C54-L78C105
						serviceFile := fmt.Sprintf("%s-%s_connectquery", protoName, service.(*build.StringExpr).Value)

						dtsOutputs = append(dtsOutputs, path.Join(f.Pkg, serviceFile))
					}
				} else {
					BazelLog.Errorf("Expected ts_proto_library.gen_connect_query_service_mapping to be a list of services, got %v", p.Value)
				}
			}
		}
	}

	if len(dtsOutputs) == 0 {
		return nil
	}

	provides := make([]resolve.ImportSpec, 0, 3*len(dtsOutputs))

	// ts_proto_library outputs both .js and .d.ts files, and can also be imported without an extension
	for _, dtsOutput := range dtsOutputs {
		provides = append(provides, resolve.ImportSpec{
			Lang: LanguageName,
			Imp:  dtsOutput,
		})
		provides = append(provides, resolve.ImportSpec{
			Lang: LanguageName,
			Imp:  dtsOutput + ".js",
		})
		provides = append(provides, resolve.ImportSpec{
			Lang: LanguageName,
			Imp:  dtsOutput + ".d.ts",
		})
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
	case NpmLinkAllKind:
		// Do not return the embedded :node_modules/{pkg} targets.
		// Instead the npm CrossResolver will resolve to specific package targets.
		break
	}

	// TODO(jbedard): ts_proto_library() embeds

	// TODO(jbedard): implement other rule kinds
	return []label.Label{}
}

var _ resolve.CrossResolver = (*typeScriptLang)(nil)

func (ts *typeScriptLang) CrossResolve(c *config.Config, ix *resolve.RuleIndex, imp resolve.ImportSpec, lang string) []resolve.FindResult {
	// Only resolve imports of js, can be from any language.
	if imp.Lang != LanguageName {
		return nil
	}

	fromRel := c.Exts[configRelExtension].(string)

	results := []resolve.FindResult{}

	// Imports of npm packages
	if impPkg, _ := node.ParseImportPath(imp.Imp); impPkg != "" {
		if pkg := ts.findPackage(fromRel, impPkg); pkg != nil {
			results = append(results, resolve.FindResult{
				Label: *pkg,
			})
		}
	}

	// Imports of js from other languages. Simulate importing from js.
	if lang != LanguageName {
		for _, r := range ix.FindRulesByImport(imp, LanguageName) {
			results = append(results, r)
		}
	}

	return results
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
			msg := fmt.Sprintf("Resolution Error: %v", err)
			fmt.Println(msg)
			BazelLog.Fatalf(msg)
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
			msg := fmt.Sprintf("Resolution Error: %v", err)
			fmt.Println(msg)
			BazelLog.Fatalf(msg)
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
		if tslibLabel := ts.findPackage(from.Pkg, "tslib"); tslibLabel != nil {
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

		// Overrides override all
		if override, ok := resolve.FindRuleWithOverride(c, imp.ImportSpec, LanguageName); ok {
			deps.Add(&override)
			continue
		}

		// JS Overrides (js_resolve) override all
		if res := cfg.GetResolution(imp.Imp); res != nil {
			deps.Add(res)
			continue
		}

		resolutionType, dep, err := ts.resolveImport(c, ix, from, imp)
		if err != nil {
			return err
		}

		types := ts.resolveImportTypes(c, ix, resolutionType, from, imp)
		for _, typesDep := range types {
			deps.Add(typesDep)
		}

		if dep != nil && (!imp.TypesOnly || len(types) == 0) {
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
	imp := impStm.ImportSpec

	// Gazelle rule index
	if resolution, match, err := ts.resolveExplicitImportFromIndex(c, ix, from, impStm); resolution != Resolution_NotFound {
		return resolution, match, err
	}

	// References to a label such as a file or file-generating rule
	if importLabel := ts.getImportLabel(imp.Imp); importLabel != nil {
		return Resolution_Label, importLabel, nil
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
			if resolution, match, err := ts.resolveExplicitImportFromIndex(c, ix, from, pImp); resolution != Resolution_NotFound {
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

func (ts *typeScriptLang) resolveExplicitImportFromIndex(
	c *config.Config,
	ix *resolve.RuleIndex,
	from label.Label,
	impStm ImportStatement) (ResolutionType, *label.Label, error) {

	matches := ix.FindRulesByImportWithConfig(c, impStm.ImportSpec, LanguageName)
	if len(matches) == 0 {
		return Resolution_NotFound, nil, nil
	}

	filteredMatches := common.NewLabelSet(from)
	for _, match := range matches {
		// Prevent from adding itself as a dependency.
		if !match.IsSelfImport(from) {
			filteredMatches.Add(&match.Label)
		}
	}

	// Too many results, don't know which is correct
	if filteredMatches.Size() > 1 {
		return Resolution_Error, nil, fmt.Errorf(
			"Import %q from %q resolved to multiple targets (%s) - this must be fixed using the \"aspect:resolve\" directive",
			impStm.ImportPath, impStm.SourcePath, targetListFromResults(matches))
	}

	// The matches were self imports, no dependency is needed
	if filteredMatches.Size() == 0 {
		return Resolution_None, nil, nil
	}

	match := filteredMatches.Labels()[0]

	BazelLog.Tracef("resolve %q import %q as %q", from, impStm.Imp, match)

	return Resolution_Label, &match, nil
}

func (ts *typeScriptLang) findPackage(from string, impPkg string) *label.Label {
	fromProject := ts.pnpmProjects.GetProject(from)
	if fromProject == nil {
		BazelLog.Tracef("resolve %q import %q project not found", from, impPkg)
		return nil
	}

	impPkgLabel := fromProject.Get(impPkg)
	if impPkgLabel == nil {
		BazelLog.Tracef("resolve %q import %q not found", from, impPkg)
		return nil
	}

	BazelLog.Tracef("resolve %q import %q to %q", from, impPkg, impPkgLabel)

	return impPkgLabel
}

func (ts *typeScriptLang) resolveImportTypes(c *config.Config, ix *resolve.RuleIndex, resolutionType ResolutionType, from label.Label, imp ImportStatement) []*label.Label {
	// Overrides are not extended with additional types
	if resolutionType == Resolution_Override {
		return nil
	}

	// The package the @types are for
	var typesPkg string
	if resolutionType == Resolution_NativeNode {
		typesPkg = "@types/node"
	} else {
		pkg, _ := node.ParseImportPath(imp.ImportSpec.Imp)
		if pkg == "" {
			return nil
		}

		typesPkg = node.ToAtTypesPackage(pkg)
	}

	typesSpec := resolve.ImportSpec{
		Lang: LanguageName,
		Imp:  typesPkg,
	}
	if matches := ix.FindRulesByImportWithConfig(c, typesSpec, LanguageName); len(matches) > 0 {
		// @types packages for any named imports
		// The import may be a package, may be an unresolved import with only @types
		return []*label.Label{&matches[0].Label}
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

// targetListFromResults returns a string with the human-readable list of
// targets contained in the given results.
func targetListFromResults(results []resolve.FindResult) string {
	list := make([]string, len(results))
	for i, result := range results {
		list[i] = result.Label.String()
	}
	return strings.Join(list, ", ")
}
