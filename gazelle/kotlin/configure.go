package gazelle

import (
	"flag"

	common "github.com/aspect-build/aspect-cli/gazelle/common"
	"github.com/aspect-build/aspect-cli/gazelle/common/git"
	"github.com/aspect-build/aspect-cli/gazelle/kotlin/kotlinconfig"
	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
	jvm_javaconfig "github.com/bazel-contrib/rules_jvm/java/gazelle/javaconfig"
	jvm_maven "github.com/bazel-contrib/rules_jvm/java/gazelle/private/maven"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"github.com/rs/zerolog"
)

var _ config.Configurer = (*kotlinLang)(nil)

func (kt *kotlinLang) KnownDirectives() []string {
	return []string{
		kotlinconfig.Directive_KotlinExtension,
		jvm_javaconfig.JavaMavenInstallFile,

		// TODO: move to common
		git.Directive_GitIgnore,
	}
}

func (kc *kotlinLang) initRootConfig(c *config.Config) kotlinconfig.Configs {
	if _, exists := c.Exts[LanguageName]; !exists {
		c.Exts[LanguageName] = kotlinconfig.Configs{
			"": kotlinconfig.New(c.RepoRoot),
		}
	}
	return c.Exts[LanguageName].(kotlinconfig.Configs)
}

func (kt *kotlinLang) Configure(c *config.Config, rel string, f *rule.File) {
	BazelLog.Tracef("Configure(%s): %s", LanguageName, rel)

	// Create the KotlinConfig for this package
	cfgs := kt.initRootConfig(c)
	cfg, exists := cfgs[rel]
	if !exists {
		parent := kotlinconfig.ParentForPackage(cfgs, rel)
		cfg = parent.NewChild(rel)
		cfgs[rel] = cfg
	}

	// Collect the ignore files for this package
	git.ReadGitConfig(c, rel, f)

	if f != nil {
		for _, d := range f.Directives {
			switch d.Key {

			case kotlinconfig.Directive_KotlinExtension:
				cfg.SetGenerationEnabled(common.ReadEnabled(d))

			// TODO: invoke java gazelle.Configure() to support all jvm directives?
			// TODO: JavaMavenRepositoryName: https://github.com/bazel-contrib/rules_jvm/commit/e46bb11bedb2ead45309eae04619caca684f6243

			case jvm_javaconfig.JavaMavenInstallFile:
				cfg.SetMavenInstallFile(d.Value)
			}
		}
	}

	if kt.mavenResolver == nil {
		BazelLog.Tracef("Creating Maven resolver: %s", cfg.MavenInstallFile())

		// TODO: better zerolog configuration
		logger := zerolog.New(BazelLog.GetOutput()).Level(zerolog.TraceLevel)

		resolver, err := jvm_maven.NewResolver(
			cfg.MavenInstallFile(),
			logger,
		)
		if err != nil {
			BazelLog.Fatalf("error creating Maven resolver: %s", err.Error())
		}
		kt.mavenResolver = &resolver
	}
}

func (kc *kotlinLang) RegisterFlags(fs *flag.FlagSet, cmd string, c *config.Config) {
	// TODO: support rules_jvm flags such as 'java-maven-install-file'? (see rules_jvm java/gazelle/configure.go)
}

func (kc *kotlinLang) CheckFlags(fs *flag.FlagSet, c *config.Config) error {
	return nil
}
