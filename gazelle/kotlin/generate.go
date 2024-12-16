package gazelle

import (
	"fmt"
	"math"
	"os"
	"path"
	"strings"
	"sync"

	gazelle "github.com/aspect-build/aspect-cli/gazelle/common"
	"github.com/aspect-build/aspect-cli/gazelle/kotlin/kotlinconfig"
	"github.com/aspect-build/aspect-cli/gazelle/kotlin/parser"
	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/bazelbuild/bazel-gazelle/resolve"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"github.com/emirpasic/gods/maps/treemap"
	"github.com/emirpasic/gods/sets/treeset"
)

const (
	// TODO: move to common
	MaxWorkerCount = 12
)

func (kt *kotlinLang) GenerateRules(args language.GenerateArgs) language.GenerateResult {
	// TODO: record args.GenFiles labels?

	cfg := args.Config.Exts[LanguageName].(kotlinconfig.Configs)[args.Rel]

	// When we return empty, we mean that we don't generate anything, but this
	// still triggers the indexing for all the TypeScript targets in this package.
	if !cfg.GenerationEnabled() {
		BazelLog.Tracef("GenerateRules(%s) disabled: %s", LanguageName, args.Rel)
		return language.GenerateResult{}
	}

	BazelLog.Tracef("GenerateRules(%s): %s", LanguageName, args.Rel)

	// Collect all source files.
	sourceFiles := kt.collectSourceFiles(cfg, args)

	// TODO: multiple library targets (lib, test, ...)
	libTarget := NewKotlinLibTarget()
	binTargets := treemap.NewWithStringComparator()

	// Parse all source files and group information into target(s)
	for p := range kt.parseFiles(args, sourceFiles) {
		var target *KotlinTarget

		if p.HasMain {
			binTarget := NewKotlinBinTarget(p.File, p.Package)
			binTargets.Put(p.File, binTarget)

			target = &binTarget.KotlinTarget
		} else {
			libTarget.Files.Add(p.File)
			libTarget.Packages.Add(p.Package)

			target = &libTarget.KotlinTarget
		}

		for _, impt := range p.Imports {
			target.Imports.Add(ImportStatement{
				ImportSpec: resolve.ImportSpec{
					Lang: LanguageName,
					Imp:  impt,
				},
				SourcePath: p.File,
			})
		}
	}

	var result language.GenerateResult

	libTargetName := gazelle.ToDefaultTargetName(args, "root")

	srcGenErr := kt.addLibraryRule(libTargetName, libTarget, args, false, &result)
	if srcGenErr != nil {
		fmt.Fprintf(os.Stderr, "Source rule generation error: %v\n", srcGenErr)
		os.Exit(1)
	}

	for _, v := range binTargets.Values() {
		binTarget := v.(*KotlinBinTarget)
		binTargetName := toBinaryTargetName(binTarget.File)
		kt.addBinaryRule(binTargetName, binTarget, args, &result)
	}

	return result
}

func (kt *kotlinLang) addLibraryRule(targetName string, target *KotlinLibTarget, args language.GenerateArgs, isTestRule bool, result *language.GenerateResult) error {
	// Check for name-collisions with the rule being generated.
	colError := gazelle.CheckCollisionErrors(targetName, KtJvmLibrary, sourceRuleKinds, args)
	if colError != nil {
		return colError
	}

	// Generate nothing if there are no source files. Remove any existing rules.
	if target.Files.Empty() {
		if args.File == nil {
			return nil
		}

		for _, r := range args.File.Rules {
			if r.Name() == targetName && r.Kind() == KtJvmLibrary {
				emptyRule := rule.NewRule(KtJvmLibrary, targetName)
				result.Empty = append(result.Empty, emptyRule)
				return nil
			}
		}

		return nil
	}

	ktLibrary := rule.NewRule(KtJvmLibrary, targetName)
	ktLibrary.SetAttr("srcs", target.Files.Values())
	ktLibrary.SetPrivateAttr(packagesKey, target)

	if isTestRule {
		ktLibrary.SetAttr("testonly", true)
	}

	result.Gen = append(result.Gen, ktLibrary)
	result.Imports = append(result.Imports, target)

	BazelLog.Infof("add rule '%s' '%s:%s'", ktLibrary.Kind(), args.Rel, ktLibrary.Name())
	return nil
}

func (kt *kotlinLang) addBinaryRule(targetName string, target *KotlinBinTarget, args language.GenerateArgs, result *language.GenerateResult) {
	main_class := strings.TrimSuffix(target.File, ".kt")
	if target.Package != "" {
		main_class = target.Package + "." + main_class
	}

	ktBinary := rule.NewRule(KtJvmBinary, targetName)
	ktBinary.SetAttr("srcs", []string{target.File})
	ktBinary.SetAttr("main_class", main_class)
	ktBinary.SetPrivateAttr(packagesKey, target)

	result.Gen = append(result.Gen, ktBinary)
	result.Imports = append(result.Imports, target)

	BazelLog.Infof("add rule '%s' '%s:%s'", ktBinary.Kind(), args.Rel, ktBinary.Name())
}

// TODO: put in common?
func (kt *kotlinLang) parseFiles(args language.GenerateArgs, sources *treeset.Set) chan *parser.ParseResult {
	// The channel of all files to parse.
	sourcePathChannel := make(chan string)

	// The channel of parse results.
	resultsChannel := make(chan *parser.ParseResult)

	// The number of workers. Don't create more workers than necessary.
	workerCount := int(math.Min(MaxWorkerCount, float64(1+sources.Size()/2)))

	// Start the worker goroutines.
	var wg sync.WaitGroup
	for i := 0; i < workerCount; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()

			for sourcePath := range sourcePathChannel {
				r, errs := parseFile(path.Join(args.Config.RepoRoot, args.Rel), sourcePath)

				// Output errors to stdout
				if len(errs) > 0 {
					fmt.Println(sourcePath, "parse error(s):")
					for _, err := range errs {
						fmt.Println(err)
					}
				}

				resultsChannel <- r
			}
		}()
	}

	// Send files to the workers.
	go func() {
		sourceFileChannelIt := sources.Iterator()
		for sourceFileChannelIt.Next() {
			sourcePathChannel <- sourceFileChannelIt.Value().(string)
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

// Parse the passed file for import statements.
func parseFile(rootDir, filePath string) (*parser.ParseResult, []error) {
	BazelLog.Tracef("ParseImports(%s): %s", LanguageName, filePath)

	content, err := os.ReadFile(path.Join(rootDir, filePath))
	if err != nil {
		return nil, []error{err}
	}

	p := parser.NewParser()
	return p.Parse(filePath, content)
}

func (kt *kotlinLang) collectSourceFiles(cfg *kotlinconfig.KotlinConfig, args language.GenerateArgs) *treeset.Set {
	sourceFiles := treeset.NewWithStringComparator()

	// TODO: "module" targets similar to java?

	gazelle.GazelleWalkDir(args, func(f string) error {
		// Otherwise the file is either source or potentially importable.
		if isSourceFileType(f) {
			BazelLog.Tracef("SourceFile: %s", f)

			sourceFiles.Add(f)
		}

		return nil
	})

	return sourceFiles
}

func isSourceFileType(f string) bool {
	ext := path.Ext(f)
	return ext == ".kt" || ext == ".kts"
}
