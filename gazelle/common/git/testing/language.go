package git_testing

import (
	"flag"

	"github.com/aspect-build/aspect-cli/gazelle/common/git"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/bazelbuild/bazel-gazelle/repo"
	"github.com/bazelbuild/bazel-gazelle/resolve"
	"github.com/bazelbuild/bazel-gazelle/rule"
)

// A noop language designed only to invoke SetupGitIgnore()

func init() {
	git.SetupGitIgnore()
}

var _ language.Language = (*gitLang)(nil)

type gitLang struct{}

// NewLanguage returns a new git language instance.
func NewLanguage() language.Language {
	return &gitLang{}
}
func (p *gitLang) Name() string                                         { return "gitignore_TESTING" }
func (p *gitLang) Configure(c *config.Config, rel string, f *rule.File) {}
func (p *gitLang) GenerateRules(args language.GenerateArgs) language.GenerateResult {
	return language.GenerateResult{}
}
func (p *gitLang) DoneGeneratingRules() {}
func (p *gitLang) Resolve(c *config.Config, ix *resolve.RuleIndex, rc *repo.RemoteCache, r *rule.Rule, imports interface{}, from label.Label) {
}
func (p *gitLang) RegisterFlags(fs *flag.FlagSet, cmd string, c *config.Config) {}
func (p *gitLang) CheckFlags(fs *flag.FlagSet, c *config.Config) error          { return nil }
func (p *gitLang) KnownDirectives() []string                                    { return nil }
func (p *gitLang) Loads() []rule.LoadInfo                                       { return nil }
func (p *gitLang) Kinds() map[string]rule.KindInfo                              { return nil }
func (p *gitLang) Fix(c *config.Config, f *rule.File)                           {}
func (p *gitLang) Embeds(r *rule.Rule, from label.Label) []label.Label          { return nil }
func (p *gitLang) Imports(c *config.Config, r *rule.Rule, f *rule.File) []resolve.ImportSpec {
	return nil
}
