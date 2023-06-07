package gazelle

import (
	"fmt"
	"math"
	"os"
	"path"
	"sync"

	gazelle "aspect.build/cli/gazelle/common"
	. "aspect.build/cli/gazelle/common/log"
	"aspect.build/cli/gazelle/kotlin/kotlinconfig"
	"aspect.build/cli/gazelle/kotlin/parser"
	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/bazelbuild/bazel-gazelle/resolve"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"github.com/emirpasic/gods/sets/treeset"
)

const (
	// TODO: move to common
	MaxWorkerCount = 12
)

func (kt *kotlinLang) GenerateRules(args language.GenerateArgs) language.GenerateResult {
	BazelLog.Tracef("GenerateRules '%s'", args.Rel)

	// TODO: record args.GenFiles labels?

	cfg := args.Config.Exts[LanguageName].(kotlinconfig.Configs)[args.Rel]

	// TODO: exit if configured to disable generation

	var result language.GenerateResult

	kt.addSourceRules(cfg, args, &result)

	return result
}

func (kt *kotlinLang) addSourceRules(cfg *kotlinconfig.KotlinConfig, args language.GenerateArgs, result *language.GenerateResult) {
	// Collect all source files.
	sourceFiles := kt.collectSourceFiles(cfg, args)

	targetName := gazelle.ToDefaultTargetName(args, "root")

	kt.addLibraryRule(targetName, sourceFiles, args, false, result)

	// TODO: test rules
}

func (kt *kotlinLang) addLibraryRule(targetName string, sourceFiles *treeset.Set, args language.GenerateArgs, isTestRule bool, result *language.GenerateResult) {
	// TODO: check for rule collisions

	// Generate nothing if there are no source files. Remove any existing rules.
	if sourceFiles.Empty() {
		if args.File == nil {
			return
		}

		for _, r := range args.File.Rules {
			if r.Name() == targetName && r.Kind() == KtJvmLibrary {
				emptyRule := rule.NewRule(KtJvmLibrary, targetName)
				result.Empty = append(result.Empty, emptyRule)
				return
			}
		}

		return
	}

	ktLibrary := rule.NewRule(KtJvmLibrary, targetName)
	ktLibrary.SetAttr("srcs", sourceFiles.Values())

	if isTestRule {
		ktLibrary.SetAttr("testonly", true)
	}

	imports := newKotlinImports()
	for impt := range kt.findAllImports(args, sourceFiles) {
		imports.Add(impt)
	}

	result.Gen = append(result.Gen, ktLibrary)
	result.Imports = append(result.Imports, imports)

	BazelLog.Infof("add rule '%s' '%s:%s'", ktLibrary.Kind(), args.Rel, ktLibrary.Name())
}

// TODO: put in common?
func (kt *kotlinLang) findAllImports(args language.GenerateArgs, sources *treeset.Set) chan ImportStatement {
	// The channel of all files to parse.
	sourcePathChannel := make(chan string)

	// The channel of parse results.
	resultsChannel := make(chan ImportStatement)

	// The number of workers. Don't create more workers than necessary.
	workerCount := int(math.Min(MaxWorkerCount, float64(1+sources.Size()/2)))

	// Start the worker goroutines.
	var wg sync.WaitGroup
	for i := 0; i < workerCount; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()

			for sourcePath := range sourcePathChannel {
				fImports, errs := parseImports(args.Config.RepoRoot, sourcePath)

				// Output errors to stdout
				if len(errs) > 0 {
					fmt.Println(sourcePath, "parse error(s):")
					for _, err := range errs {
						fmt.Println(err)
					}
				}

				// Emit import package + paths
				for _, sourceImport := range fImports {
					resultsChannel <- ImportStatement{
						ImportSpec: resolve.ImportSpec{
							Imp:  sourceImport,
							Lang: LanguageName,
						},
						SourcePath: sourcePath,
					}
				}
			}
		}()
	}

	// Send files to the workers.
	go func() {
		sourceFileChannelIt := sources.Iterator()
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

// Parse the passed file for import statements.
func parseImports(rootDir, filePath string) ([]string, []error) {
	BazelLog.Debugf("ParseImports: %s", filePath)

	content, err := os.ReadFile(path.Join(rootDir, filePath))
	if err != nil {
		return nil, []error{err}
	}

	p := parser.NewParser()
	return p.ParseImports(filePath, string(content))
}

func (kt *kotlinLang) collectSourceFiles(cfg *kotlinconfig.KotlinConfig, args language.GenerateArgs) *treeset.Set {
	sourceFiles := treeset.NewWithStringComparator()

	// TODO: "module" targets similar to java?

	gazelle.GazelleWalkDir(args, false, func(f string) error {
		// Globally managed file ignores.
		if kt.gitignore.Matches(path.Join(args.Rel, f)) {
			BazelLog.Tracef("File git ignored: %s / %s", args.Rel, f)

			return nil
		}

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
