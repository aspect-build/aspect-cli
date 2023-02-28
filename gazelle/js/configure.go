package gazelle

import (
	"flag"
	"fmt"
	"log"
	"os"
	"path"
	"strconv"
	"strings"

	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/rule"
)

// Configurer satisfies the config.Configurer interface. It's the
// language-specific configuration extension.
type Configurer struct {
	config.Configurer
}

// RegisterFlags registers command-line flags used by the extension. This
// method is called once with the root configuration when Gazelle
// starts. RegisterFlags may set an initial values in Config.Exts. When flags
// are set, they should modify these values.
func (ts *Configurer) RegisterFlags(fs *flag.FlagSet, cmd string, c *config.Config) {}

// CheckFlags validates the configuration after command line flags are parsed.
// This is called once with the root configuration when Gazelle starts.
// CheckFlags may set default values in flags or make implied changes.
func (ts *Configurer) CheckFlags(fs *flag.FlagSet, c *config.Config) error {
	return nil
}

// KnownDirectives returns a list of directive keys that this Configurer can
// interpret. Gazelle prints errors for directives that are not recoginized by
// any Configurer.
func (ts *Configurer) KnownDirectives() []string {
	return []string{
		Directive_TypeScriptExtension,
		Directive_GenerationMode,
		Directive_Lockfile,
		Directive_IgnoreImports,
		Directive_Resolve,
		Directive_ValidateImportStatements,
		Directive_LibraryNamingConvention,
		Directive_TestsNamingConvention,
		Directive_NpmPackageNameConvention,
		Directive_LibraryFiles,
		Directive_TestFiles,
	}
}

// Configure modifies the configuration using directives and other information
// extracted from a build file. Configure is called in each directory.
//
// c is the configuration for the current directory. It starts out as a copy
// of the configuration for the parent directory.
//
// rel is the slash-separated relative path from the repository root to
// the current directory. It is "" for the root directory itself.
//
// f is the build file for the current directory or nil if there is no
// existing build file.
func (ts *TypeScript) Configure(c *config.Config, rel string, f *rule.File) {
	BazelLog.Tracef("Configure %s", rel)

	// Create the root config.
	if _, exists := c.Exts[LanguageName]; !exists {
		c.Exts[LanguageName] = NewConfigs()
	}

	if f != nil {
		ts.readDirectives(c, rel, f)
	}

	ts.collectIgnoreFiles(c, rel)

	ts.readWorkspaces(c, rel)
}

func (ts *TypeScript) collectIgnoreFiles(c *config.Config, rel string) {
	cfgs := c.Exts[LanguageName].(Configs)

	// Collect gitignore style ignore files in this directory.
	for _, ignoreFileName := range bazelIgnoreFiles {
		ignoreRelPath := path.Join(rel, ignoreFileName)
		ignoreFilePath := path.Join(c.RepoRoot, ignoreRelPath)

		if _, ignoreErr := os.Stat(ignoreFilePath); ignoreErr == nil {
			BazelLog.Tracef("Add ignore file %s", ignoreRelPath)

			ignoreErr := cfgs.gitignore.AddIgnoreFile(rel, ignoreFilePath)
			if ignoreErr != nil {
				log.Fatalf("Failed to add ignore file %s: %v", ignoreRelPath, ignoreErr)
			}
		}
	}
}

func (ts *TypeScript) readWorkspaces(c *config.Config, rel string) {
	configs := c.Exts[LanguageName].(Configs)
	config := configs.Get(rel)

	lockfilePath := path.Join(c.RepoRoot, rel, config.PnpmLockfile())
	if _, err := os.Stat(lockfilePath); err == nil {
		ts.addPnpmLockfile(config, c.RepoName, c.RepoRoot, path.Join(rel, config.PnpmLockfile()))
	}
}

func (ts *TypeScript) readDirectives(c *config.Config, rel string, f *rule.File) {
	configs := c.Exts[LanguageName].(Configs)
	config := configs.Get(rel)

	for _, d := range f.Directives {
		value := strings.TrimSpace(d.Value)

		switch d.Key {
		case "exclude":
			// We record the exclude directive since we do manual tree traversal of subdirs.
			config.AddExcludedPattern(value)
		case Directive_TypeScriptExtension:
			switch d.Value {
			case "enabled":
				config.SetGenerationEnabled(true)
			case "disabled":
				config.SetGenerationEnabled(false)
			default:
				err := fmt.Errorf("invalid value for directive %q: %s: possible values are enabled/disabled",
					Directive_TypeScriptExtension, d.Value)
				log.Fatal(err)
			}
		case Directive_GenerationMode:
			mode := GenerationModeType(strings.TrimSpace(d.Value))
			switch mode {
			case GenerationModeDirectory:
				config.SetGenerationMode(mode)
			case GenerationModeNone:
				config.SetGenerationMode(mode)
			default:
				log.Fatalf("invalid value for directive %q: %s", Directive_GenerationMode, d.Value)
			}
		case Directive_Lockfile:
			config.SetPnpmLockfile(value)
		case Directive_IgnoreImports:
			config.AddIgnoredImport(strings.TrimSpace(value))
		case Directive_Resolve:
			globTarget := strings.Split(value, " ")
			if len(globTarget) != 2 {
				err := fmt.Errorf("invalid value for directive %q: %s: value must be filename/glob + label",
					Directive_Resolve, d.Value)
				log.Fatal(err)
			}

			label, labelErr := label.Parse(strings.TrimSpace(globTarget[1]))
			if labelErr != nil {
				err := fmt.Errorf("invalid label for directive %q: %s",
					Directive_Resolve, label)
				log.Fatal(err)
			}

			config.AddResolve(strings.TrimSpace(globTarget[0]), &label)
		case Directive_ValidateImportStatements:
			v, err := strconv.ParseBool(value)
			if err != nil {
				log.Fatal(err)
			}
			config.SetValidateImportStatements(v)
		case Directive_LibraryNamingConvention:
			config.SetLibraryNamingConvention(value)
		case Directive_TestsNamingConvention:
			config.SetTestsNamingLibraryConvention(value)
		case Directive_NpmPackageNameConvention:
			config.SetNpmPackageNamingConvention(value)
		case Directive_LibraryFiles:
			config.AddLibraryFileGlob(value)
		case Directive_TestFiles:
			config.AddTestFileGlob(value)
		}
	}
}
