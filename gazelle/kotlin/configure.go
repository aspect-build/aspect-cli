package gazelle

import (
	"flag"

	common "aspect.build/cli/gazelle/common"
	"aspect.build/cli/gazelle/kotlin/kotlinconfig"
	BazelLog "aspect.build/cli/pkg/logger"
	jvm_javaconfig "github.com/bazel-contrib/rules_jvm/java/gazelle/javaconfig"
	jvm_maven "github.com/bazel-contrib/rules_jvm/java/gazelle/private/maven"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"github.com/rs/zerolog"
)

type Configurer struct {
	config.Configurer

	lang *kotlinLang

	mavenInstallFile string
}

func NewConfigurer(lang *kotlinLang) *Configurer {
	return &Configurer{
		lang: lang,
	}
}

func (kt *Configurer) KnownDirectives() []string {
	return []string{
		kotlinconfig.Directive_KotlinExtension,
		jvm_javaconfig.JavaMavenInstallFile,
	}
}

func (kc *Configurer) initRootConfig(c *config.Config) kotlinconfig.Configs {
	if _, exists := c.Exts[LanguageName]; !exists {
		c.Exts[LanguageName] = kotlinconfig.Configs{
			"": kotlinconfig.New(c.RepoRoot),
		}
	}
	return c.Exts[LanguageName].(kotlinconfig.Configs)
}

func (kt *Configurer) Configure(c *config.Config, rel string, f *rule.File) {
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
	kt.lang.gitignore.CollectIgnoreFiles(c, rel)

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

	if kt.lang.mavenResolver == nil {
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
		kt.lang.mavenResolver = &resolver
	}
}

func (kc *Configurer) RegisterFlags(fs *flag.FlagSet, cmd string, c *config.Config) {
	// TODO: support rules_jvm flags such as 'java-maven-install-file'? (see rules_jvm java/gazelle/configure.go)
}

func (kc *Configurer) CheckFlags(fs *flag.FlagSet, c *config.Config) error {
	return nil
}
