package gazelle

import (
	"fmt"
	"strings"
	"sync"
	_ "unsafe"

	"github.com/bazelbuild/bazel-gazelle/label"
	_ "github.com/bazelbuild/bazel-gazelle/walk"
	"github.com/bmatcuk/doublestar/v4"
	"github.com/emirpasic/gods/maps/linkedhashmap"
	"github.com/emirpasic/gods/sets/treeset"
)

// Directives. Keep in sync with documentation in cli/core/docs/help/topics/directives.md
const (
	// Directive_TypeScriptExtension represents the directive that controls whether
	// this TypeScript generation is enabled or not. Sub-packages inherit this value.
	// Can be either "enabled" or "disabled". Defaults to "enabled".
	Directive_TypeScriptExtension = "js"
	// Directive_GenerationMode represents the directive that controls the BUILD generation
	// mode. See below for the GenerationModeType constants.
	Directive_GenerationMode = "js_generation_mode"
	// The pnpm-lock.yaml file.
	Directive_Lockfile = "js_pnpm_lockfile"
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
	// The glob for the main library files, excludes files matching Directive_TestFiles.
	Directive_LibraryFiles = "js_files"
	// The glob for test files.
	Directive_TestFiles = "js_test_files"
	// A TypeScript tsconfig.json file path for typescript compilation config.
	Directive_JsGazelleConfigJson = "js_tsconfig"
)

// GenerationModeType represents one of the generation modes.
type GenerationModeType string

// Generation modes
const (
	GenerationModeNone GenerationModeType = "none"
	// GenerationModeDirectory defines the mode in which a coarse-grained target will
	// be generated for each sub-directory.
	GenerationModeDirectory GenerationModeType = "directory"
)

const (
	DefaultNpmLinkAllTargetName = "node_modules"
	TargetNameDirectoryVar      = "{dirname}"
	DefaultLibraryName          = TargetNameDirectoryVar
	DefaultTestsName            = TargetNameDirectoryVar + "_tests"
	NpmPackageContentName       = TargetNameDirectoryVar + "_lib"

	// The suffix added to the end of a target being wrapped in a package.
	PackageSrcSuffix = "_lib"

	// The default should align with the rules_js default npm_translate_lock(npm_package_target_name)
	DefaultNpmPackageTargetName = TargetNameDirectoryVar
)

var DefaultSourceGlobs = map[string][]string{
	DefaultLibraryName: {fmt.Sprintf("**/*.{%s}", strings.Join(sourceFileExtensionsArray, ","))},
	DefaultTestsName:   {fmt.Sprintf("**/*.{spec,test}.{%s}", strings.Join(sourceFileExtensionsArray, ","))},
}

var (
	// BUILD file names.
	buildFileNames = []string{"BUILD", "BUILD.bazel"}

	// Ignore files following .gitignore syntax for files gazelle will ignore.
	bazelIgnoreFiles = []string{".bazelignore", ".gitignore"}

	// Set of supported source file extensions.
	sourceFileExtensions = treeset.NewWithStringComparator("ts", "tsx", "mts", "cts")

	// Array of sourceFileExtensions.
	sourceFileExtensionsArray = []string{"ts", "tsx", "mts", "cts"}

	// Importable declaration files that are not compiled
	declarationFileExtensionsArray = []string{"d.ts", "d.mts", "d.cts"}

	// Supported data file extensions that typescript can reference.
	dataFileExtensions = treeset.NewWithStringComparator("json")

	// The default TypeScript config file name
	defaultTsConfig = "tsconfig.json"
)

// JsGazelleConfig represents a config extension for a specific Bazel package.
type JsGazelleConfig struct {
	rel    string
	parent *JsGazelleConfig

	generationEnabled bool
	generationMode    GenerationModeType

	pnpmLockPath string

	excludes                 []string
	ignoreDependencies       []string
	resolves                 *linkedhashmap.Map
	validateImportStatements bool
	fileTypeGlobs            map[string][]string

	// Generated rule names
	npmLinkAllTargetName       string
	libraryNamingConvention    string
	testsNamingConvention      string
	npmPackageNamingConvention string

	// Name/location of tsconfig files relative to BUILDs
	tsconfigName string
}

// New creates a new JsGazelleConfig.
func newRootConfig() *JsGazelleConfig {
	return &JsGazelleConfig{
		rel:                        "",
		generationEnabled:          true,
		generationMode:             GenerationModeDirectory,
		pnpmLockPath:               "pnpm-lock.yaml",
		excludes:                   make([]string, 0),
		ignoreDependencies:         make([]string, 0),
		resolves:                   linkedhashmap.New(),
		validateImportStatements:   true,
		npmLinkAllTargetName:       DefaultNpmLinkAllTargetName,
		npmPackageNamingConvention: DefaultNpmPackageTargetName,
		libraryNamingConvention:    DefaultLibraryName,
		testsNamingConvention:      DefaultTestsName,
		fileTypeGlobs:              make(map[string][]string),
		tsconfigName:               defaultTsConfig,
	}
}

// NewChild creates a new child JsGazelleConfig. It inherits desired values from the
// current JsGazelleConfig and sets itself as the parent to the child.
func (c *JsGazelleConfig) NewChild(childPath string) *JsGazelleConfig {
	cCopy := *c
	cCopy.rel = childPath
	cCopy.parent = c
	cCopy.excludes = make([]string, 0)
	cCopy.ignoreDependencies = make([]string, 0)
	cCopy.resolves = linkedhashmap.New()
	cCopy.fileTypeGlobs = make(map[string][]string)
	return &cCopy
}

// AddExcludedPattern adds a glob pattern parsed from the standard gazelle:exclude directive.
func (c *JsGazelleConfig) AddExcludedPattern(pattern string) {
	c.excludes = append(c.excludes, pattern)
}

// Determine if the file path is ignored based on the current configuration.
func (c *JsGazelleConfig) IsFileExcluded(fileRelPath string) bool {
	// Gazelle exclude directive.
	wc := &walkConfig{excludes: c.excludes}

	return isExcluded(wc, c.rel, fileRelPath)
}

// Required for using go:linkname below for using the private isExcluded.
// https://github.com/bazelbuild/bazel-gazelle/blob/v0.28.0/walk/config.go#L54-L73
type walkConfig struct {
	excludes []string
	// Below are fields that are not used by the isExcluded function but match the walkConfig
	// upstream walk.(*walkConfig).
	_ bool      // ignore bool
	_ []string  // follow []string
	_ sync.Once // loadOnce sync.Once
}

//go:linkname isExcluded github.com/bazelbuild/bazel-gazelle/walk.(*walkConfig).isExcluded
func isExcluded(wc *walkConfig, rel, base string) bool

// SetGenerationEnabled sets whether the extension is enabled or not.
func (c *JsGazelleConfig) SetGenerationEnabled(enabled bool) {
	c.generationEnabled = enabled
}

// GenerationEnabled returns whether the extension is enabled or not.
func (c *JsGazelleConfig) GenerationEnabled() bool {
	return c.generationEnabled
}

// Set the pnpm-workspace.yaml file path.
func (c *JsGazelleConfig) SetPnpmLockfile(pnpmLockPath string) {
	c.pnpmLockPath = pnpmLockPath
}
func (c *JsGazelleConfig) PnpmLockfile() string {
	return c.pnpmLockPath
}

// Adds a dependency to the list of ignored dependencies for
// a given package. Adding an ignored dependency to a package also makes it
// ignored on a subpackage.
func (c *JsGazelleConfig) AddIgnoredImport(impGlob string) {
	c.ignoreDependencies = append(c.ignoreDependencies, impGlob)
}

// Checks if a import is ignored in the given package or
// in one of the parent packages up to the workspace root.
func (c *JsGazelleConfig) IsImportIgnored(impt string) bool {
	config := c
	for config != nil {
		for _, glob := range config.ignoreDependencies {
			m, e := doublestar.Match(glob, impt)

			if e != nil {
				fmt.Println("Ignore import glob error: ", e)
				return false
			}

			if m {
				return true
			}
		}

		config = config.parent
	}

	return false
}

func (c *JsGazelleConfig) AddResolve(imprt string, label *label.Label) {
	c.resolves.Put(imprt, label)
}

func (c *JsGazelleConfig) GetResolution(imprt string) *label.Label {
	config := c
	for config != nil {
		for _, glob := range config.resolves.Keys() {
			m, e := doublestar.Match(glob.(string), imprt)
			if e != nil {
				fmt.Println("Resolve import glob error: ", e)
				return nil
			}

			if m {
				resolveLabel, _ := config.resolves.Get(glob)
				return resolveLabel.(*label.Label)
			}
		}
		config = config.parent
	}

	return nil
}

// SetValidateImportStatements sets whether TypeScript import statements should be
// validated or not. It throws an error if this is set multiple times, i.e. if
// the directive is specified multiple times in the Bazel workspace.
func (c *JsGazelleConfig) SetValidateImportStatements(validate bool) {
	c.validateImportStatements = validate
}

// ValidateImportStatements returns whether the TypeScript import statements should
// be validated or not. If this option was not explicitly specified by the user,
// it defaults to true.
func (c *JsGazelleConfig) ValidateImportStatements() bool {
	return c.validateImportStatements
}

// SetGenerationMode sets whether coarse-grained targets should be
// generated or not.
func (c *JsGazelleConfig) SetGenerationMode(generationMode GenerationModeType) {
	c.generationMode = generationMode
}

// GenerationMode returns whether coarse-grained targets should be
// generated or not.
func (c *JsGazelleConfig) GenerationMode() GenerationModeType {
	return c.generationMode
}

// SetLibraryNamingConvention sets the ts_project target naming convention.
func (c *JsGazelleConfig) SetLibraryNamingConvention(libraryNamingConvention string) {
	c.libraryNamingConvention = libraryNamingConvention
}

// SetTestsNamingLibraryConvention sets the ts_project test target naming convention.
func (c *JsGazelleConfig) SetTestsNamingLibraryConvention(testsNamingConvention string) {
	c.testsNamingConvention = testsNamingConvention
}

func (c *JsGazelleConfig) SetNpmPackageNamingConvention(testsNamingConvention string) {
	c.npmPackageNamingConvention = testsNamingConvention
}

// The library name when wrapped within an npm package.
func (c *JsGazelleConfig) RenderNpmSourceLibraryName(npmPackageName string) string {
	return npmPackageName + PackageSrcSuffix
}

// renderTargetName returns the ts_project target name by performing all substitutions.
func (c *JsGazelleConfig) RenderTargetName(name, packageName string) string {
	return strings.ReplaceAll(name, TargetNameDirectoryVar, packageName)
}

// AddLibraryFileGlob sets the glob used to find source files.
func (c *JsGazelleConfig) AddLibraryFileGlob(srcsFileGlob string) {
	c.addFilePathGlob(DefaultLibraryName, srcsFileGlob)
}

// IsSourceFile determines if a given file path is considered a source file.
// See AddLibraryFileGlob().
func (c *JsGazelleConfig) IsSourceFile(filePath string) bool {
	if !isSourceFileType(filePath) {
		return false
	}

	return c.filePathMatches(DefaultLibraryName, filePath)
}

// AddTestFileGlob sets the glob used to find test source files.
func (c *JsGazelleConfig) AddTestFileGlob(testsFileGlob string) {
	c.addFilePathGlob(DefaultTestsName, testsFileGlob)
}

// IsTestFile determines if a given file path is considered a test source file.
// See AddTestFileGlob().
func (c *JsGazelleConfig) IsTestFile(filePath string) bool {
	if !isSourceFileType(filePath) {
		return false
	}

	return c.filePathMatches(DefaultTestsName, filePath)
}

func (c *JsGazelleConfig) GetSourceGroups() []string {
	return []string{DefaultLibraryName, DefaultTestsName}
}

func (c *JsGazelleConfig) addFilePathGlob(srcType, glob string) {
	if c.fileTypeGlobs[srcType] == nil {
		c.fileTypeGlobs[srcType] = make([]string, 1)
	}
	c.fileTypeGlobs[srcType] = append(c.fileTypeGlobs[srcType], glob)
}

func (c *JsGazelleConfig) filePathMatches(srcType, filePath string) bool {
	// Find the first config containing globs for srcType
	globConfig := c
	for globConfig != nil && globConfig.fileTypeGlobs[srcType] == nil {
		globConfig = globConfig.parent
	}

	// Extract the globs or use the defaults
	var globs []string
	if globConfig == nil {
		globs = DefaultSourceGlobs[srcType]
	} else {
		globs = globConfig.fileTypeGlobs[srcType]
	}

	// This type has no globs or defaults
	if globs == nil {
		BazelLog.Debugf("Source type '%s' has no globs", srcType)
		return false
	}

	// Test for any match
	for _, g := range globs {
		m, e := doublestar.Match(g, filePath)
		if e != nil {
			fmt.Printf("Source glob '%s' error: %v", g, e)
			return false
		}

		if m {
			return true
		}
	}

	return false
}

func (c *JsGazelleConfig) SetTsconfigName(tsconfigName string) {
	c.tsconfigName = tsconfigName
}
