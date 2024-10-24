package gazelle

import (
	"flag"
	"fmt"
	"log"
	"os"
	"path"
	"strings"

	common "aspect.build/cli/gazelle/common"
	"aspect.build/cli/gazelle/common/git"
	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/rule"
)

// Configurer satisfies the config.Configurer interface. It's the
// language-specific configuration extension.
var _ config.Configurer = (*typeScriptLang)(nil)

// RegisterFlags registers command-line flags used by the extension. This
// method is called once with the root configuration when Gazelle
// starts. RegisterFlags may set an initial values in Config.Exts. When flags
// are set, they should modify these values.
func (ts *typeScriptLang) RegisterFlags(fs *flag.FlagSet, cmd string, c *config.Config) {
}

// CheckFlags validates the configuration after command line flags are parsed.
// This is called once with the root configuration when Gazelle starts.
// CheckFlags may set default values in flags or make implied changes.
func (ts *typeScriptLang) CheckFlags(fs *flag.FlagSet, c *config.Config) error {
	return nil
}

// KnownDirectives returns a list of directive keys that this Configurer can
// interpret. Gazelle prints errors for directives that are not recoginized by
// any Configurer.
func (ts *typeScriptLang) KnownDirectives() []string {
	return []string{
		Directive_TypeScriptExtension,
		Directive_TypeScriptProtoExtension,
		Directive_TypeScriptConfigExtension,
		Directive_NpmPackageExtension,
		Directive_GenerationMode,
		Directive_Visibility,
		Directive_Lockfile,
		Directive_IgnoreImports,
		Directive_Resolve,
		Directive_ValidateImportStatements,
		Directive_LibraryNamingConvention,
		Directive_TestsNamingConvention,
		Directive_NpmPackageNameConvention,
		Directive_PackageRuleKind,
		Directive_LibraryFiles,
		Directive_TestFiles,

		// TODO(deprecated): remove
		Directive_CustomTargetFiles,
		Directive_CustomTargetTestFiles,

		// TODO: move to common
		git.Directive_GitIgnore,
		common.Directive_GenerationMode,
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
func (ts *typeScriptLang) Configure(c *config.Config, rel string, f *rule.File) {
	BazelLog.Tracef("Configure(%s): %s", LanguageName, rel)

	// Create the root config.
	if cfg, exists := c.Exts[LanguageName]; !exists {
		c.Exts[LanguageName] = newRootConfig()
	} else {
		c.Exts[LanguageName] = cfg.(*JsGazelleConfig).NewChild(rel)
	}

	if f != nil {
		ts.readDirectives(c, rel, f)

		// Read configurations relative to the current BUILD file.
		ts.readConfigurations(c, rel)
	}

	common.ReadWalkConfig(c, rel, f)

	git.ReadGitConfig(c, rel, f)
}

func (ts *typeScriptLang) readConfigurations(c *config.Config, rel string) {
	config := c.Exts[LanguageName].(*JsGazelleConfig)

	// pnpm
	lockfilePath := path.Join(c.RepoRoot, rel, config.PnpmLockfile())
	if _, err := os.Stat(lockfilePath); err == nil {
		ts.addPnpmLockfile(config, c.RepoName, c.RepoRoot, path.Join(rel, config.PnpmLockfile()))
	}

	// tsconfig
	// TODO: add support for alternate tsconfig names
	configPath := path.Join(c.RepoRoot, rel, config.defaultTsconfigName)
	if _, err := os.Stat(configPath); err == nil {
		ts.tsconfig.AddTsConfigFile(c.RepoRoot, rel, config.defaultTsconfigName)
	}
}

func (ts *typeScriptLang) readDirectives(c *config.Config, rel string, f *rule.File) {
	config := c.Exts[LanguageName].(*JsGazelleConfig)

	for _, d := range f.Directives {
		value := strings.TrimSpace(d.Value)

		switch d.Key {
		case Directive_TypeScriptExtension:
			config.SetGenerationEnabled(common.ReadEnabled(d))
		case Directive_TypeScriptConfigExtension:
			config.SetTsConfigGenerationEnabled(common.ReadEnabled(d))
		case Directive_TypeScriptProtoExtension:
			config.SetProtoGenerationEnabled(common.ReadEnabled(d))
		case Directive_NpmPackageExtension:
			if strings.TrimSpace(d.Value) == string(NpmPackageReferencedMode) {
				config.SetNpmPackageGenerationMode(NpmPackageReferencedMode)
			} else if common.ReadEnabled(d) {
				config.SetNpmPackageGenerationMode(NpmPackageEnabledMode)
			} else {
				config.SetNpmPackageGenerationMode(NpmPackageDisabledMode)
			}
		case Directive_Visibility:
			group := DefaultLibraryName

			// A list of all non-empty labels
			visLabels := make([]string, 0, 1)
			for _, visLabel := range strings.Split(value, " ") {
				visLabel = strings.TrimSpace(visLabel)
				if visLabel != "" {
					visLabels = append(visLabels, visLabel)
				}
			}

			// The first entry may be the group-key, not a label
			if len(visLabels) > 0 && !(strings.HasPrefix(visLabels[0], ":") || strings.HasPrefix(visLabels[0], "//")) {
				group = visLabels[0]
				visLabels = visLabels[1:]
			}

			config.SetVisibility(group, visLabels)

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
			switch value {
			case "error":
				config.SetValidateImportStatements(ValidationError)
			case "warn":
				config.SetValidateImportStatements(ValidationWarn)
			case "off":
				config.SetValidateImportStatements(ValidationOff)
			default:
				log.Fatalf("invalid value for directive %q: %s", Directive_ValidateImportStatements, d.Value)
			}
		case Directive_LibraryNamingConvention:
			config.SetLibraryNamingConvention(value)
		case Directive_TestsNamingConvention:
			config.SetTestsNamingLibraryConvention(value)
		case Directive_NpmPackageNameConvention:
			config.SetNpmPackageNamingConvention(value)
		case Directive_PackageRuleKind:
			switch value {
			case JsLibraryKind:
				config.packageTargetKind = JsLibraryKind
			case NpmPackageKind:
				config.packageTargetKind = NpmPackageKind
			default:
				log.Fatalf("invalid value for directive %q: %s", Directive_PackageRuleKind, d.Value)
			}
		case Directive_LibraryFiles:
			group := DefaultLibraryName
			groupGlob := value

			if i := strings.Index(value, " "); i != -1 {
				group = value[:i]
				groupGlob = strings.TrimSpace(value[i+1:])
			}

			config.addTargetGlob(group, groupGlob, false)
		case Directive_TestFiles:
			group := DefaultTestsName
			groupGlob := value

			if i := strings.Index(value, " "); i != -1 {
				group = value[:i]
				groupGlob = strings.TrimSpace(value[i+1:])
			}

			config.addTargetGlob(group, groupGlob, true)

		// TODO: remove, deprecated
		case Directive_CustomTargetFiles:
			fmt.Fprintf(os.Stderr, "DEPRECATED: %s is deprecated, use %s\n", Directive_CustomTargetFiles, Directive_LibraryFiles)
			os.Exit(1)
		case Directive_CustomTargetTestFiles:
			fmt.Fprintf(os.Stderr, "DEPRECATED: %s is deprecated, use %s\n", Directive_CustomTargetTestFiles, Directive_TestFiles)
			os.Exit(1)
		case Directive_GenerationMode:
			fmt.Fprintf(os.Stderr, "DEPRECATED: %s is deprecated, use %s %s|%s\n", Directive_GenerationMode, common.Directive_GenerationMode, common.GenerationModeUpdate, common.GenerationModeCreate)
			os.Exit(1)
		}
	}
}
