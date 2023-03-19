package gazelle

import (
	pnpm "aspect.build/cli/gazelle/js/pnpm"
	"aspect.build/cli/gazelle/js/typescript"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/language"
)

const LanguageName = "js"

// The Gazelle extension for TypeScript rules.
// TypeScript satisfies the language.Language interface including the
// Configurer and Resolver types.
type TypeScript struct {
	Configurer
	Resolver
	Language

	// Importable files and the generating label.
	fileLabels map[string]*label.Label

	// Importable npm-like packages. Each pnpm project has its own set
	// of importable npm packages.
	// BUILDs alongside pnpm project roots have a map. BUILDs within a project contain a reference
	// to the parent pnpm project map.
	pnpmProjects *pnpm.PnpmProjectMap

	// TypeScript configuration across the workspace
	tsconfig *typescript.TsWorkspace
}

// NewLanguage initializes a new TypeScript that satisfies the language.Language
// interface. This is the entrypoint for the extension initialization.
func NewLanguage() language.Language {
	return &TypeScript{
		fileLabels:   make(map[string]*label.Label),
		pnpmProjects: pnpm.NewPnpmProjectMap(),
		tsconfig:     typescript.NewTsWorkspace(),
	}
}
