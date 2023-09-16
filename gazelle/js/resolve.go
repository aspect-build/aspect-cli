package gazelle

import (
	"fmt"
	"os"
	"path"
	"strings"
	"time"

	common "aspect.build/cli/gazelle/common"
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

// Resolver satisfies the resolve.Resolver interface. It resolves dependencies
// in rules generated by this extension.
type Resolver struct {
	lang *typeScriptLang
}

func NewResolver(lang *typeScriptLang) resolve.Resolver {
	return &Resolver{
		lang: lang,
	}
}

const (
	Resolution_Error      = -1
	Resolution_None       = 0
	Resolution_NotFound   = 1
	Resolution_Package    = 2
	Resolution_Label      = 3
	Resolution_NativeNode = 4
	Resolution_Other      = 5
)

type ResolutionType = int

// Name returns the name of the language. This is the prefix of the kinds of
// rules generated. E.g. ts_project
func (*Resolver) Name() string { return LanguageName }

// Determine what rule (r) outputs which can be imported.
// For TypeScript this is all the import-paths pointing to files within the rule.
func (ts *Resolver) Imports(c *config.Config, r *rule.Rule, f *rule.File) []resolve.ImportSpec {
	BazelLog.Debugf("Imports '%s:%s'", f.Pkg, r.Name())

	switch r.Kind() {
	case TsProtoLibraryKind:
		return ts.protoLibraryImports(r, f)
	case TsConfigKind:
		return ts.tsconfigImports(r, f)
	default:
		return ts.sourceFileImports(r, f)
	}
}

// TypeScript-importable ImportSpecs from a set of source files.
func (ts *Resolver) sourceFileImports(r *rule.Rule, f *rule.File) []resolve.ImportSpec {
	srcs := r.AttrStrings("srcs")

	provides := make([]resolve.ImportSpec, 0, len(srcs)+1)

	// Sources that produce importable paths.
	for _, src := range srcs {
		src = path.Clean(path.Join(f.Pkg, src))

		for _, impt := range toImportPaths(src) {
			provides = append(provides, resolve.ImportSpec{
				Lang: LanguageName,
				Imp:  impt,
			})
		}
	}

	if len(provides) == 0 {
		return nil
	}

	return provides
}

func (ts *Resolver) tsconfigImports(r *rule.Rule, f *rule.File) []resolve.ImportSpec {
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
func (ts *Resolver) protoLibraryImports(r *rule.Rule, f *rule.File) []resolve.ImportSpec {
	protoSrcs := r.PrivateAttr("proto_library_srcs").([]string)
	provides := make([]resolve.ImportSpec, 0, len(protoSrcs)+1)

	for _, src := range protoSrcs {
		src = path.Clean(path.Join(f.Pkg, src))

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
func (ts *Resolver) Embeds(r *rule.Rule, from label.Label) []label.Label {
	BazelLog.Debugf("Embeds '%s' rules", from.String())

	switch r.Kind() {
	case TsProjectKind:
		srcs := r.AttrStrings("srcs")
		tsEmbeds := make([]label.Label, 0, len(srcs))

		// The compiled dts and js files are accessible as embedded rules
		for _, src := range srcs {
			if isTranspiledSourceFileType(src) {
				js := swapSourceExtension(src)
				dts := path.Base(js) + ".d" + path.Ext(js)
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
func (ts *Resolver) Resolve(
	c *config.Config,
	ix *resolve.RuleIndex,
	rc *repo.RemoteCache,
	r *rule.Rule,
	importData interface{},
	from label.Label,
) {
	start := time.Now()
	BazelLog.Infof("Resolve '%s' dependencies", from.String())

	// TsProject imports are resolved as deps
	if r.Kind() == TsProjectKind || r.Kind() == JsLibraryKind || r.Kind() == TsConfigKind || r.Kind() == TsProtoLibraryKind {
		deps, err := ts.resolveModuleDeps(c, ix, importData.(*TsProjectInfo).imports, from)
		if err != nil {
			BazelLog.Fatalf("Resolution Error: ", err)
			os.Exit(1)
		}

		if !deps.Empty() {
			r.SetAttr("deps", deps.Labels())
		}
	}

	BazelLog.Infof("Resolve '%s' DONE in %s", from.String(), time.Since(start).String())
}

func (ts *Resolver) resolveModuleDeps(
	c *config.Config,
	ix *resolve.RuleIndex,
	modules *treeset.Set,
	from label.Label,
) (*common.LabelSet, error) {
	cfg := c.Exts[LanguageName].(*JsGazelleConfig)

	deps := common.NewLabelSet(from)
	resolutionErrors := []error{}

	it := modules.Iterator()
	for it.Next() {
		mod := it.Value().(ImportStatement)

		resolutionType, dep, err := ts.resolveModuleDep(c, ix, mod, from)
		if err != nil {
			return nil, err
		}

		if resolutionType == Resolution_NotFound {
			// The import itself was not found, but a type definition was found for it
			if types := ts.resolveImportTypes(from, mod.Imp); len(types) > 0 {
				for _, typesDep := range types {
					deps.Add(typesDep)
				}
				continue
			}

			if cfg.ValidateImportStatements() != ValidationOff {
				BazelLog.Debugf("import '%s' for target '%s' not found", mod.ImportPath, from.String())

				notFound := fmt.Errorf(
					"Import %[1]q from %[2]q is an unknown dependency. Possible solutions:\n"+
						"\t1. Instruct Gazelle to resolve to a known dependency using a directive:\n"+
						"\t\t# gazelle:resolve [src-lang] js import-string label\n"+
						"\t\t   or\n"+
						"\t\t# gazelle:js_resolve import-string-glob label\n"+
						"\t2. Ignore the dependency using the '# gazelle:%[3]s %[1]s' directive.\n"+
						"\t3. Disable Gazelle resolution validation using '# gazelle:%[4]s off'",
					mod.ImportPath, mod.SourcePath, Directive_IgnoreImports, Directive_ValidateImportStatements,
				)
				resolutionErrors = append(resolutionErrors, notFound)
			}

			continue
		}

		if dep != nil {
			deps.Add(dep)
		}

		// Add any relevant type definitions such as @types packages
		if resolutionType == Resolution_NativeNode {
			if typesNode := ts.resolveAtTypes(from, "node"); typesNode != nil {
				deps.Add(typesNode)
			}
		} else if resolutionType == Resolution_Package {
			for _, typesDep := range ts.resolveImportTypes(from, mod.Imp) {
				deps.Add(typesDep)
			}
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

	return deps, nil
}

func (ts *Resolver) resolveModuleDep(
	c *config.Config,
	ix *resolve.RuleIndex,
	mod ImportStatement,
	from label.Label,
) (ResolutionType, *label.Label, error) {
	cfg := c.Exts[LanguageName].(*JsGazelleConfig)

	imp := mod.ImportSpec

	// Overrides
	if override, ok := resolve.FindRuleWithOverride(c, imp, LanguageName); ok {
		return Resolution_Other, &override, nil
	}

	// JS Overrides (js_resolve)
	if res := cfg.GetResolution(imp.Imp); res != nil {
		return Resolution_Label, res, nil
	}

	possible := make([]resolve.ImportSpec, 0, 1)
	possible = append(possible, mod.ImportSpec)
	for _, expandedImp := range mod.Alt {
		possible = append(possible, resolve.ImportSpec{Lang: mod.Lang, Imp: expandedImp})
	}

	// Gazelle rule index. Try each potential expanded path
	for _, eImp := range possible {
		if matches := ix.FindRulesByImportWithConfig(c, eImp, LanguageName); len(matches) > 0 {
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
					"Import %q from %q resolved to multiple targets (%s)"+
						" - this must be fixed using the \"gazelle:resolve\" directive",
					mod.ImportPath, mod.SourcePath, targetListFromResults(matches))
			}

			// The matches were self imports, no dependency is needed
			if len(filteredMatches) == 0 {
				return Resolution_None, nil, nil
			}

			relMatch := filteredMatches[0].Rel(from.Repo, from.Pkg)

			return Resolution_Other, &relMatch, nil
		}
	}

	// References to a label such as a file or file-generating rule
	if importLabel := ts.lang.GetImportLabel(imp.Imp); importLabel != nil {
		relImport := importLabel.Rel(from.Repo, from.Pkg)

		return Resolution_Label, &relImport, nil
	}

	// References to an npm package, pnpm workspace projects etc.
	if pkg := ts.resolvePackage(from, mod.Imp); pkg != nil {
		return Resolution_Package, pkg, nil
	}

	// Native node imports
	if node.IsNodeImport(imp.Imp) {
		return Resolution_NativeNode, nil, nil
	}

	return Resolution_NotFound, nil, nil
}

func (ts *Resolver) resolvePackage(from label.Label, imp string) *label.Label {
	// Imports of npm-like packages
	// Trim to only the package name or scoped package name
	parts := strings.SplitN(imp, "/", 2)
	if parts[0][0] == "@"[0] {
		parts[1] = strings.SplitN(parts[1], "/", 2)[0]
	} else {
		parts = parts[0:1]
	}

	impPkg := path.Join(parts...)

	fromProject := ts.lang.pnpmProjects.GetProject(from.Pkg)
	if fromProject == nil {
		BazelLog.Tracef("resolve '%s' import '%s' project not found", from.String(), impPkg)
		return nil
	}

	impPkgLabel := fromProject.Get(impPkg)
	if impPkgLabel == nil {
		BazelLog.Tracef("resolve '%s' (project '%s') import '%s' not found", from.String(), from.Pkg, impPkg)
		return nil
	}

	relPkgLabel := impPkgLabel.Rel(from.Repo, from.Pkg)

	BazelLog.Tracef("resolve '%s' (project '%s') import '%s' to '%s'", from.String(), from.Pkg, impPkg, relPkgLabel)

	return &relPkgLabel
}

func (ts *Resolver) resolveImportTypes(from label.Label, imp string) []*label.Label {
	deps := make([]*label.Label, 0)

	// @types packages for the import
	if typesPkg := ts.resolveAtTypes(from, imp); typesPkg != nil {
		deps = append(deps, typesPkg)
	}

	// Additional module definitions for the import
	if typeModules := ts.lang.moduleTypes[imp]; typeModules != nil {
		for _, mod := range typeModules {
			relMode := mod.Rel(from.Repo, from.Pkg)
			deps = append(deps, &relMode)
		}
	}

	return deps
}

func toAtTypesPackage(imp string) string {
	pkgParts := strings.Split(imp, "/")

	if imp[0] == '@' {
		if len(pkgParts) < 2 {
			BazelLog.Errorf("Invalid scoped package: '%s'", imp)
			return ""
		}

		pkgParts = []string{pkgParts[0][1:], pkgParts[1]}
	} else {
		pkgParts = []string{pkgParts[0]}
	}

	return path.Join("@types", strings.Join(pkgParts, "__"))
}

// Find and resolve any @types package for an import
func (ts *Resolver) resolveAtTypes(from label.Label, imp string) *label.Label {
	fromProject := ts.lang.pnpmProjects.GetProject(from.Pkg)
	if fromProject == nil {
		return nil
	}

	typesPkg := toAtTypesPackage(imp)
	typesPkgLabel := fromProject.Get(typesPkg)
	if typesPkgLabel == nil {
		return nil
	}

	relImpPkgLabel := typesPkgLabel.Rel(from.Repo, from.Pkg)
	return &relImpPkgLabel
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
