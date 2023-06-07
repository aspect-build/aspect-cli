package gazelle

import (
	git "aspect.build/cli/gazelle/common/git"
	pnpm "aspect.build/cli/gazelle/js/pnpm"
	"aspect.build/cli/gazelle/js/typescript"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/bazelbuild/bazel-gazelle/resolve"
)

const LanguageName = "js"

// The Gazelle extension for TypeScript rules.
// TypeScript satisfies the language.Language interface including the
// Configurer and Resolver types.
type typeScriptLang struct {
	config.Configurer
	resolve.Resolver

	// Importable files and the generating label.
	fileLabels map[string]*label.Label

	// Importable npm-like packages. Each pnpm project has its own set
	// of importable npm packages.
	// BUILDs alongside pnpm project roots have a map. BUILDs within a project contain a reference
	// to the parent pnpm project map.
	pnpmProjects *pnpm.PnpmProjectMap

	// TypeScript configuration across the workspace
	tsconfig *typescript.TsWorkspace

	// Ignore configurations for the workspace.
	gitignore *git.GitIgnore
}

// NewLanguage initializes a new TypeScript that satisfies the language.Language
// interface. This is the entrypoint for the extension initialization.
func NewLanguage() language.Language {
	l := typeScriptLang{
		fileLabels:   make(map[string]*label.Label),
		pnpmProjects: pnpm.NewPnpmProjectMap(),
		tsconfig:     typescript.NewTsWorkspace(),
		gitignore:    git.NewGitIgnore(),
	}

	l.Configurer = NewConfigurer(&l)
	l.Resolver = NewResolver(&l)

	return &l
}
