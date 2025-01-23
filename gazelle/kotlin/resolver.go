package gazelle

import (
	"fmt"
	"log"
	"os"
	"strings"
	"time"

	common "github.com/aspect-build/aspect-cli/gazelle/common"
	"github.com/aspect-build/aspect-cli/gazelle/kotlin/kotlinconfig"
	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/repo"
	"github.com/bazelbuild/bazel-gazelle/resolve"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"github.com/emirpasic/gods/sets/treeset"

	jvm_types "github.com/bazel-contrib/rules_jvm/java/gazelle/private/types"
)

var _ resolve.Resolver = (*kotlinLang)(nil)

const (
	Resolution_Error        = -1
	Resolution_None         = 0
	Resolution_NotFound     = 1
	Resolution_Label        = 2
	Resolution_NativeKotlin = 3
)

type ResolutionType = int

func (*kotlinLang) Name() string {
	return LanguageName
}

// Determine what rule (r) outputs which can be imported.
func (kt *kotlinLang) Imports(c *config.Config, r *rule.Rule, f *rule.File) []resolve.ImportSpec {
	BazelLog.Debugf("Imports(%s): '%s:%s'", LanguageName, f.Pkg, r.Name())

	if r.PrivateAttr(packagesKey) != nil {
		target, isLib := r.PrivateAttr(packagesKey).(*KotlinLibTarget)
		if isLib {
			provides := make([]resolve.ImportSpec, 0, target.Packages.Size())
			for _, pkg := range target.Packages.Values() {
				provides = append(provides, resolve.ImportSpec{
					Lang: LanguageName,
					Imp:  pkg.(string),
				})
			}

			if len(provides) > 0 {
				return provides
			}
		}
	}

	return nil
}

func (kt *kotlinLang) Embeds(r *rule.Rule, from label.Label) []label.Label {
	return []label.Label{}
}

func (kt *kotlinLang) Resolve(c *config.Config, ix *resolve.RuleIndex, rc *repo.RemoteCache, r *rule.Rule, importData interface{}, from label.Label) {
	start := time.Now()
	BazelLog.Infof("Resolve(%s): //%s:%s", LanguageName, from.Pkg, r.Name())

	if r.Kind() == KtJvmLibrary || r.Kind() == KtJvmBinary {
		var target KotlinTarget

		if r.Kind() == KtJvmLibrary {
			target = importData.(*KotlinLibTarget).KotlinTarget
		} else {
			target = importData.(*KotlinBinTarget).KotlinTarget
		}

		deps, err := kt.resolveImports(c, ix, target.Imports, from)
		if err != nil {
			log.Fatalf("Resolution Error: %v", err)
			os.Exit(1)
		}

		if !deps.Empty() {
			r.SetAttr("deps", deps.Labels())
		}
	}

	BazelLog.Infof("Resolve(%s): //%s:%s DONE in %s", LanguageName, from.Pkg, r.Name(), time.Since(start).String())
}

func (kt *kotlinLang) resolveImports(
	c *config.Config,
	ix *resolve.RuleIndex,
	imports *treeset.Set,
	from label.Label,
) (*common.LabelSet, error) {
	deps := common.NewLabelSet(from)

	it := imports.Iterator()
	for it.Next() {
		mod := it.Value().(ImportStatement)

		resolutionType, dep, err := kt.resolveImport(c, ix, mod, from)
		if err != nil {
			return nil, err
		}

		if resolutionType == Resolution_NotFound {
			BazelLog.Debugf("import '%s' for target '%s' not found", mod.Imp, from.String())

			notFound := fmt.Errorf(
				"Import %[1]q from %[2]q is an unknown dependency. Possible solutions:\n"+
					"\t1. Instruct Gazelle to resolve to a known dependency using a directive:\n"+
					"\t\t# aspect:resolve [src-lang] kotlin import-string label\n",
				mod.Imp, mod.SourcePath,
			)

			fmt.Printf("Resolution error %v\n", notFound)
			continue
		}

		if resolutionType == Resolution_NativeKotlin || resolutionType == Resolution_None {
			continue
		}

		if dep != nil {
			deps.Add(dep)
		}
	}

	return deps, nil
}

func (kt *kotlinLang) resolveImport(
	c *config.Config,
	ix *resolve.RuleIndex,
	impt ImportStatement,
	from label.Label,
) (ResolutionType, *label.Label, error) {
	imptSpec := impt.ImportSpec

	// Gazelle overrides
	// TODO: generalize into gazelle/common
	if override, ok := resolve.FindRuleWithOverride(c, imptSpec, LanguageName); ok {
		return Resolution_Label, &override, nil
	}

	// TODO: generalize into gazelle/common
	if matches := ix.FindRulesByImportWithConfig(c, imptSpec, LanguageName); len(matches) > 0 {
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
					" - this must be fixed using the \"aspect:resolve\" directive",
				impt.Imp, impt.SourcePath, targetListFromResults(matches))
		}

		// The matches were self imports, no dependency is needed
		if len(filteredMatches) == 0 {
			return Resolution_None, nil, nil
		}

		match := filteredMatches[0]

		return Resolution_Label, &match, nil
	}

	// Native kotlin imports
	if IsNativeImport(impt.Imp) {
		return Resolution_NativeKotlin, nil, nil
	}

	jvm_import := jvm_types.NewPackageName(impt.Imp)

	cfgs := c.Exts[LanguageName].(kotlinconfig.Configs)
	cfg, _ := cfgs[from.Pkg]

	// Maven imports
	if mavenResolver := kt.mavenResolver; mavenResolver != nil {
		if l, mavenError := (*mavenResolver).Resolve(jvm_import, cfg.ExcludedArtifacts(), cfg.MavenRepositoryName()); mavenError == nil {
			return Resolution_Label, &l, nil
		} else {
			BazelLog.Debugf("Maven resolution failed: %v", mavenError)
		}
	}

	// The original import, like "x.y.z" might be a subpackage within a package that resolves,
	// so try to resolve the original identifer, then try to resolve the parent
	// identifier, etc.
	importParent := impt.packageFullyQualifiedName().Parent()
	if importParent == nil {
		return Resolution_NotFound, nil, nil
	}
	parentImportSpec := impt
	parentImportSpec.Imp = importParent.String()
	return kt.resolveImport(c, ix, parentImportSpec, from)
}

// targetListFromResults returns a string with the human-readable list of
// targets contained in the given results.
// TODO: move to gazelle/common
func targetListFromResults(results []resolve.FindResult) string {
	list := make([]string, len(results))
	for i, result := range results {
		list[i] = result.Label.String()
	}
	return strings.Join(list, ", ")
}
