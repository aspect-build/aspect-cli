package gazelle

import (
	"fmt"
	"log"
	"os"
	"path"
	"strings"

	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bmatcuk/doublestar/v4"
	"github.com/emirpasic/gods/maps/linkedhashmap"
)

// Directives. Keep in sync with documentation in /docs/cli/help/directives.md
const (
	// Directive_TypeScriptExtension represents the directive that controls whether
	// this TypeScript generation is enabled or not. Sub-packages inherit this value.
	// Can be either "enabled" or "disabled". Defaults to "enabled".
	Directive_TypeScriptExtension = "js"
	// En/disable ts_proto_library generation
	Directive_TypeScriptProtoExtension = "js_proto"
	// En/disable ts_config generation
	Directive_TypeScriptConfigExtension = "js_tsconfig"
	// En/disable npm_package generation and when generate
	Directive_NpmPackageExtension = "js_npm_package"
	// Visibility of the primary library targets
	Directive_Visibility = "js_visibility"
	// The pnpm-lock.yaml file.
	Directive_Lockfile = "js_pnpm_lockfile"
	// The tsconfig.json file.
	Directive_TsconfigFile = "js_tsconfig_file"
	// Ignore and do not generate a tsconfig related `ts_project` attribute
	Directive_TypeScriptConfigIgnore = "js_tsconfig_ignore"
	// Directive_IgnoreImports represents the directive that controls the
	// ignored dependencies from the generated targets.
	// Sub-packages extend this value.
	// Ignored imports may be file path globs.
	Directive_IgnoreImports = "js_ignore_imports"
	// Directive_Resolve represents a gazelle:resolve state which supports globs.
	Directive_Resolve = "js_resolve"
	// Directive_ValidateImportStatements represents the directive that controls
	// whether the TypeScript import statements should be validated.
	Directive_ValidateImportStatements = "js_validate_import_statements"
	// Directive_LibraryNamingConvention represents the directive that controls the
	// ts_project naming convention. It interpolates {dirname} with the
	// Bazel package name. E.g. if the Bazel package name is `foo`, setting this
	// to `{dirname}_my_lib` would render to `foo_my_lib`.
	Directive_LibraryNamingConvention = "js_project_naming_convention"
	// The target name for npm_package() rules. See npm_translate_lock(npm_package_target_name)
	Directive_NpmPackageNameConvention = "js_npm_package_target_name"
	// Directive_TestsNamingConvention represents the directive that controls the ts_project test
	// naming convention. See js_project_naming_convention for more info on
	// the package name interpolation.
	Directive_TestsNamingConvention = "js_tests_naming_convention"
	// Use of npm_package() vs js_library() for the linked npm package target
	Directive_PackageRuleKind = "js_package_rule_kind"
	// The glob for the main library files, excludes files matching Directive_TestFiles.
	Directive_LibraryFiles = "js_files"
	// The glob for test files.
	Directive_TestFiles = "js_test_files"

	// TODO(deprecated): remove - replaced with js_files [group]
	Directive_CustomTargetFiles = "js_custom_files"
	// TODO(deprecated): remove - replaced with js_test_files [group]
	Directive_CustomTargetTestFiles = "js_custom_test_files"
	// TODO(deprecated): remove - replaced with common generation_mode
	Directive_JsGenerationMode = "js_generation_mode"
)

type NpmPackageMode string

const (
	NpmPackageEnabledMode    NpmPackageMode = "enabled"
	NpmPackageDisabledMode   NpmPackageMode = "disabled"
	NpmPackageReferencedMode NpmPackageMode = "referenced"
)

const (
	DefaultNpmLinkAllTargetName = "node_modules"
	TargetNameDirectoryVar      = "{dirname}"
	DefaultLibraryName          = TargetNameDirectoryVar
	DefaultTestsName            = TargetNameDirectoryVar + "_tests"

	// The default name for ts_proto_library rules, where {proto_library} is the name of the proto target
	ProtoNameVar            = "{proto_library}"
	DefaultProtoLibraryName = ProtoNameVar + "_ts"

	// The suffix added to the end of a target being wrapped in a package.
	PackageSrcSuffix = "_lib"

	// The default should align with the rules_js default npm_translate_lock(npm_package_target_name)
	DefaultNpmPackageTargetName = TargetNameDirectoryVar
)

type TargetGroup struct {
	// The target name template of the target group.
	// Supports {dirname} variable.
	name string

	// Custom glob patterns for sources.
	customSources []string

	// Default glob patterns for sources. Only set for pre-configured targets.
	defaultSources []string

	// The `visibility` of the target
	visibility []label.Label

	// If the targets are always testonly
	testonly bool
}

var DefaultSourceGlobs = []*TargetGroup{
	&TargetGroup{
		name:           DefaultLibraryName,
		customSources:  []string{},
		defaultSources: []string{fmt.Sprintf("%s/**/*.{%s}", rootDirVar, strings.Join(defaultTypescriptFileExtensionsArray, ","))},
		testonly:       false,
	},
	&TargetGroup{
		name:           DefaultTestsName,
		customSources:  []string{},
		defaultSources: []string{fmt.Sprintf("%s/**/*.{spec,test}.{%s}", rootDirVar, strings.Join(defaultTypescriptFileExtensionsArray, ","))},
		testonly:       true,
	},
}

var (
	rootDirVar = "${rootDir}"

	// Array of default typescript source file extensions
	defaultTypescriptFileExtensionsArray = []string{"ts", "tsx", "mts", "cts"}
)

// ValidationMode represents what should happen when validation errors are found.
type ValidationMode int

const (
	// ValidationError has gazelle produce an error when validation errors are found.
	ValidationError ValidationMode = iota
	// ValidationWarn has gazelle print warnings when validation errors are found.
	ValidationWarn
	// ValidationOff has gazelle swallow validation errors silently.
	ValidationOff
)

type PackageTargetKind string

const (
	PackageTargetKind_Package PackageTargetKind = NpmPackageKind
	PackageTargetKind_Library PackageTargetKind = JsLibraryKind
)

// JsGazelleConfig represents a config extension for a specific Bazel package.
type JsGazelleConfig struct {
	rel    string
	parent *JsGazelleConfig

	generationEnabled bool

	packageTargetKind PackageTargetKind

	protoGenerationEnabled    bool
	tsconfigGenerationEnabled bool
	packageGenerationEnabled  NpmPackageMode

	pnpmLockPath string

	tsconfigName         string
	tsconfigIgnoredProps []string

	ignoreDependencies       []string
	resolves                 *linkedhashmap.Map
	validateImportStatements ValidationMode
	targets                  []*TargetGroup

	// Generated rule names
	npmLinkAllTargetName       string
	targetNamingOverrides      map[string]string
	npmPackageNamingConvention string
	tsProtoLibraryName         string
}

// New creates a new JsGazelleConfig.
func newRootConfig() *JsGazelleConfig {
	return &JsGazelleConfig{
		rel:                        "",
		generationEnabled:          true,
		protoGenerationEnabled:     true,
		tsconfigGenerationEnabled:  true,
		packageGenerationEnabled:   NpmPackageReferencedMode,
		packageTargetKind:          PackageTargetKind_Package,
		pnpmLockPath:               "pnpm-lock.yaml",
		tsconfigName:               "tsconfig.json",
		ignoreDependencies:         make([]string, 0),
		tsconfigIgnoredProps:       make([]string, 0),
		resolves:                   linkedhashmap.New(),
		validateImportStatements:   ValidationError,
		npmLinkAllTargetName:       DefaultNpmLinkAllTargetName,
		npmPackageNamingConvention: DefaultNpmPackageTargetName,
		targetNamingOverrides:      make(map[string]string),
		tsProtoLibraryName:         DefaultProtoLibraryName,
		targets:                    DefaultSourceGlobs[:],
	}
}

func (g *TargetGroup) newChild() *TargetGroup {
	sources := g.customSources
	if len(sources) == 0 {
		sources = g.defaultSources
	}

	return &TargetGroup{
		name:           g.name,
		customSources:  make([]string, 0),
		defaultSources: sources,
		testonly:       g.testonly,
		visibility:     g.visibility,
	}
}

// NewChild creates a new child JsGazelleConfig. It inherits desired values from the
// current JsGazelleConfig and sets itself as the parent to the child.
func (c *JsGazelleConfig) NewChild(childPath string) *JsGazelleConfig {
	cCopy := *c
	cCopy.rel = childPath
	cCopy.parent = c
	cCopy.ignoreDependencies = make([]string, 0)
	cCopy.resolves = linkedhashmap.New()

	// Copy the targets, any modifications will be local.
	cCopy.targets = make([]*TargetGroup, 0, len(c.targets))
	for _, target := range c.targets {
		cCopy.targets = append(cCopy.targets, target.newChild())
	}

	// Copy the overrides, any modifications will be local.
	cCopy.targetNamingOverrides = make(map[string]string)
	for k, v := range c.targetNamingOverrides {
		cCopy.targetNamingOverrides[k] = v
	}

	return &cCopy
}

// Render a target name by applying target name tmeplate vars
func (c *JsGazelleConfig) renderTargetName(baseName, packageName string) string {
	return strings.ReplaceAll(baseName, TargetNameDirectoryVar, packageName)
}

func (c *JsGazelleConfig) SetVisibility(groupName string, visLabels []string) {
	target := c.GetSourceTarget(groupName)
	if target == nil {
		log.Fatal(fmt.Errorf("Target %s not found in %s", groupName, c.rel))
		os.Exit(1)
	}

	target.visibility = make([]label.Label, 0, len(visLabels))

	for _, v := range visLabels {
		l, labelErr := label.Parse(strings.TrimSpace(v))
		if labelErr != nil {
			log.Fatal(fmt.Errorf("invalid label for visibility: %s", l))
			os.Exit(1)
		}

		target.visibility = append(target.visibility, l)
	}
}

// SetGenerationEnabled sets whether the extension is enabled or not.
func (c *JsGazelleConfig) SetGenerationEnabled(enabled bool) {
	c.generationEnabled = enabled
}

// GenerationEnabled returns whether the extension is enabled or not.
func (c *JsGazelleConfig) GenerationEnabled() bool {
	return c.generationEnabled
}

func (c *JsGazelleConfig) SetTsConfigGenerationEnabled(enabled bool) {
	c.tsconfigGenerationEnabled = enabled
}

// If ts_config extension is enabled.
func (c *JsGazelleConfig) GetTsConfigGenerationEnabled() bool {
	return c.tsconfigGenerationEnabled
}

func (c *JsGazelleConfig) SetProtoGenerationEnabled(enabled bool) {
	c.protoGenerationEnabled = enabled
}

// If ts_proto_library extension is enabled.
func (c *JsGazelleConfig) ProtoGenerationEnabled() bool {
	return c.generationEnabled && c.protoGenerationEnabled
}

func (c *JsGazelleConfig) SetNpmPackageGenerationMode(mode NpmPackageMode) {
	c.packageGenerationEnabled = mode
}

func (c *JsGazelleConfig) GetNpmPackageGenerationMode() NpmPackageMode {
	return c.packageGenerationEnabled
}

// Set the pnpm-workspace.yaml file path.
func (c *JsGazelleConfig) SetPnpmLockfile(pnpmLockPath string) {
	c.pnpmLockPath = path.Clean(pnpmLockPath)
}
func (c *JsGazelleConfig) PnpmLockfile() string {
	return c.pnpmLockPath
}

// Set the tsconfig.json file name
func (c *JsGazelleConfig) SetTsconfigFile(tsconfigName string) {
	c.tsconfigName = tsconfigName
}
func (c *JsGazelleConfig) GetTsconfigFile() string {
	return c.tsconfigName
}

func (c *JsGazelleConfig) AddIgnoredTsConfig(propName string) {
	// TODO: potentially support multiple comma-separated properties, removing properties instead of only adding

	for _, prop := range tsProjectReflectedConfigAttributes {
		if prop == propName {
			c.tsconfigIgnoredProps = append(c.tsconfigIgnoredProps, propName)
			return
		}
	}

	fmt.Printf("Unknown ts_project attribute to ignore: %q\n\nIgnored attributes must be the ts_project attribute, not the tsconfig.json option name\n", propName)
}

func (c *JsGazelleConfig) IsTsConfigIgnored(propName string) bool {
	for _, prop := range c.tsconfigIgnoredProps {
		if prop == propName {
			return true
		}
	}
	return false
}

// Adds a dependency to the list of ignored dependencies for
// a given package. Adding an ignored dependency to a package also makes it
// ignored on a subpackage.
func (c *JsGazelleConfig) AddIgnoredImport(impGlob string) {
	if !doublestar.ValidatePattern(impGlob) {
		fmt.Println("Invalid js ignore import glob: ", impGlob)
		return
	}

	c.ignoreDependencies = append(c.ignoreDependencies, impGlob)
}

// Checks if a import is ignored in the given package or
// in one of the parent packages up to the workspace root.
func (c *JsGazelleConfig) IsImportIgnored(impt string) bool {
	config := c
	for config != nil {
		for _, glob := range config.ignoreDependencies {
			if doublestar.MatchUnvalidated(glob, impt) {
				return true
			}
		}

		config = config.parent
	}

	return false
}

func (c *JsGazelleConfig) AddResolve(imprt string, label *label.Label) {
	if !doublestar.ValidatePattern(imprt) {
		fmt.Println("Invalid js resolve glob: ", imprt)
		return
	}

	c.resolves.Put(imprt, label)
}

func (c *JsGazelleConfig) GetResolution(imprt string) *label.Label {
	config := c
	for config != nil {
		for _, glob := range config.resolves.Keys() {
			if doublestar.MatchUnvalidated(glob.(string), imprt) {
				resolveLabel, _ := config.resolves.Get(glob)
				return resolveLabel.(*label.Label)
			}
		}
		config = config.parent
	}

	return nil
}

// SetValidateImportStatements sets the ValidationMode for TypeScript import
// statements. It throws an error if this is set multiple times, i.e. if the
// directive is specified multiple times in the Bazel workspace.
func (c *JsGazelleConfig) SetValidateImportStatements(mode ValidationMode) {
	c.validateImportStatements = mode
}

// ValidateImportStatements returns whether the TypeScript import statements should
// be validated or not. If this option was not explicitly specified by the user,
// it defaults to true.
func (c *JsGazelleConfig) ValidateImportStatements() ValidationMode {
	return c.validateImportStatements
}

// SetLibraryNamingConvention sets the ts_project target naming convention.
func (c *JsGazelleConfig) SetLibraryNamingConvention(libraryNamingConvention string) {
	c.targetNamingOverrides[DefaultLibraryName] = libraryNamingConvention
}

// SetTestsNamingLibraryConvention sets the ts_project test target naming convention.
func (c *JsGazelleConfig) SetTestsNamingLibraryConvention(testsNamingConvention string) {
	c.targetNamingOverrides[DefaultTestsName] = testsNamingConvention
}

func (c *JsGazelleConfig) MapTargetName(name string) string {
	if c.targetNamingOverrides[name] != "" {
		return c.targetNamingOverrides[name]
	}
	return name
}

func (c *JsGazelleConfig) SetNpmPackageNamingConvention(testsNamingConvention string) {
	c.npmPackageNamingConvention = testsNamingConvention
}

func (c *JsGazelleConfig) RenderNpmPackageTargetName(packageName string) string {
	return c.renderTargetName(c.npmPackageNamingConvention, packageName)
}

func (c *JsGazelleConfig) SetTsProtoLibraryNamingConvention(tsProtoLibraryName string) {
	c.tsProtoLibraryName = tsProtoLibraryName
}

func (c *JsGazelleConfig) RenderTsProtoLibraryName(protoLibraryName string) string {
	return strings.ReplaceAll(c.tsProtoLibraryName, ProtoNameVar, protoLibraryName)
}

func (c *JsGazelleConfig) RenderTsConfigName(tsconfigName string) string {
	return strings.ReplaceAll(strings.TrimRight(path.Base(tsconfigName), ".json"), ".", "_")
}

// Returns the ts_project target name by performing all substitutions.
func (c *JsGazelleConfig) RenderSourceTargetName(groupName, packageName string, hasPackageTarget bool) string {
	mappedGroupName := c.MapTargetName(groupName)
	ruleName := c.renderTargetName(mappedGroupName, packageName)

	// If rendering the default library and both the library and npm_package
	// names still have the default configuration then the npm_package takes
	// that default name. The library name then gets the "npm source library"
	// target name.
	if hasPackageTarget && groupName == DefaultLibraryName && mappedGroupName == DefaultLibraryName {
		npmPackageRuleName := c.RenderNpmPackageTargetName(packageName)
		if ruleName == npmPackageRuleName {
			return ruleName + PackageSrcSuffix
		}
	}

	return ruleName
}

// Determine if and which target the passed file belongs in.
func (c *JsGazelleConfig) GetFileSourceTarget(filePath, rootDir string) *TargetGroup {
	// Rules are evaluated in reverse order, so we want to
	for i := len(c.targets) - 1; i >= 0; i-- {
		target := c.targets[i]
		sources := target.customSources

		// Fallback to default sources if no sources are specified
		if len(sources) == 0 {
			sources = target.defaultSources
		}

		for _, globTmpl := range sources {
			glob := path.Clean(strings.Replace(globTmpl, rootDirVar, rootDir, 1))

			if doublestar.MatchUnvalidated(glob, filePath) {
				return target
			}
		}
	}

	return nil
}

// Return a list of all source groups for this config, including primary library + tests.
// The list is the source of truth for the order of groups
func (c *JsGazelleConfig) GetSourceTargets() []*TargetGroup {
	return c.targets
}
func (c *JsGazelleConfig) GetSourceTarget(targetName string) *TargetGroup {
	// Update existing target with the same name
	for _, target := range c.targets {
		if target.name == targetName {
			return target
		}
	}

	return nil
}

// AddTargetGlob sets the glob used to find source files for the specified target
func (c *JsGazelleConfig) addTargetGlob(targetName, glob string, isTestOnly bool) {
	// Update existing target with the same name
	for _, target := range c.targets {
		if target.name == targetName {
			if target.testonly != isTestOnly {
				targetWord := "non-test"
				overrideWord := "test"
				if target.testonly {
					targetWord = "test"
					overrideWord = "non-test"
				}
				log.Fatalf("Custom %s target %s:%s can not override %s target", targetWord, c.rel, targetName, overrideWord)
				os.Exit(1)
			}

			if !doublestar.ValidatePattern(glob) {
				log.Fatalf("Invalid target (%s) glob: %v", target.name, glob)
				os.Exit(1)
				return
			}

			target.customSources = append(target.customSources, glob)
			return
		}
	}

	// ... otherwise create a new target
	c.targets = append(c.targets, &TargetGroup{
		name:          targetName,
		customSources: []string{glob},
		testonly:      isTestOnly,
	})
}
