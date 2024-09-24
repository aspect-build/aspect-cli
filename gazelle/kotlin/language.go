package gazelle

import (
	jvm_maven "github.com/bazel-contrib/rules_jvm/java/gazelle/private/maven"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"github.com/emirpasic/gods/sets/treeset"
)

const LanguageName = "kotlin"

const (
	KtJvmLibrary              = "kt_jvm_library"
	KtJvmBinary               = "kt_jvm_binary"
	RulesKotlinRepositoryName = "io_bazel_rules_kotlin"
)

var sourceRuleKinds = treeset.NewWithStringComparator(KtJvmLibrary)

var _ language.Language = (*kotlinLang)(nil)

// The Gazelle extension for TypeScript rules.
// TypeScript satisfies the language.Language interface including the
// Configurer and Resolver types.
type kotlinLang struct {
	// TODO: extend rules_jvm extension instead of duplicating?
	mavenResolver    *jvm_maven.Resolver
	mavenInstallFile string
}

// NewLanguage initializes a new TypeScript that satisfies the language.Language
// interface. This is the entrypoint for the extension initialization.
func NewLanguage() language.Language {
	return &kotlinLang{}
}

var kotlinKinds = map[string]rule.KindInfo{
	KtJvmLibrary: {
		MatchAny: false,
		NonEmptyAttrs: map[string]bool{
			"srcs": true,
		},
		SubstituteAttrs: map[string]bool{},
		MergeableAttrs: map[string]bool{
			"srcs": true,
		},
		ResolveAttrs: map[string]bool{
			"deps": true,
		},
	},

	KtJvmBinary: {
		MatchAny: false,
		NonEmptyAttrs: map[string]bool{
			"srcs":       true,
			"main_class": true,
		},
		SubstituteAttrs: map[string]bool{},
		MergeableAttrs:  map[string]bool{},
		ResolveAttrs:    map[string]bool{},
	},
}

var kotlinLoads = []rule.LoadInfo{
	{
		Name: "@" + RulesKotlinRepositoryName + "//kotlin:jvm.bzl",
		Symbols: []string{
			KtJvmLibrary,
			KtJvmBinary,
		},
	},
}

func (*kotlinLang) Kinds() map[string]rule.KindInfo {
	return kotlinKinds
}

func (*kotlinLang) Loads() []rule.LoadInfo {
	return kotlinLoads
}

func (*kotlinLang) Fix(c *config.Config, f *rule.File) {}
