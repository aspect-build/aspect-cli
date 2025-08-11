package gazelle

import (
	pnpm "github.com/aspect-build/aspect-cli/gazelle/languages/js/pnpm"
	"github.com/aspect-build/aspect-cli/gazelle/languages/js/typescript"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/language"
)

const LanguageName = "js"

var _ language.Language = (*typeScriptLang)(nil)

// The Gazelle extension for TypeScript rules.
// TypeScript satisfies the language.Language interface including the
// Configurer and Resolver types.
type typeScriptLang struct {
	// Importable files and the generating label.
	fileLabels map[string]*label.Label

	// Importable type definitions and the generating labels.
	// Multiple labels may define/extend the same type definition, potentially also extending packages.
	moduleTypes map[string][]*label.Label

	// Importable npm-like packages. Each pnpm project has its own set
	// of importable npm packages.
	// BUILDs alongside pnpm project roots have a map. BUILDs within a project contain a reference
	// to the parent pnpm project map.
	pnpmProjects *pnpm.PnpmProjectMap

	// TypeScript configuration across the workspace
	tsconfig *typescript.TsWorkspace
}

var _ language.Language = (*typeScriptLang)(nil)
var _ language.ModuleAwareLanguage = (*typeScriptLang)(nil)

// NewLanguage initializes a new TypeScript that satisfies the language.Language
// interface. This is the entrypoint for the extension initialization.
func NewLanguage() language.Language {
	pnpmProjects := pnpm.NewPnpmProjectMap()

	return &typeScriptLang{
		fileLabels:   make(map[string]*label.Label),
		moduleTypes:  make(map[string][]*label.Label),
		pnpmProjects: pnpmProjects,
		tsconfig:     typescript.NewTsWorkspace(pnpmProjects),
	}
}
