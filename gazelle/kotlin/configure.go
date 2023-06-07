package gazelle

import (
	"flag"

	. "aspect.build/cli/gazelle/common/log"
	"aspect.build/cli/gazelle/kotlin/kotlinconfig"
	"github.com/bazel-contrib/rules_jvm/java/gazelle/javaconfig"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/rule"
)

type Configurer struct {
	config.Configurer

	lang *kotlinLang
}

func NewConfigurer(lang *kotlinLang) *Configurer {
	return &Configurer{
		lang: lang,
	}
}

func (kt *Configurer) KnownDirectives() []string {
	return []string{
		javaconfig.JavaMavenInstallFile,
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
	BazelLog.Tracef("Configure %s", rel)

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

			// TODO: invoke java gazelle.Configure()?
			case javaconfig.JavaMavenInstallFile:
				cfg.SetMavenInstallFile(d.Value)
			}
		}
	}
}

func (kc *Configurer) RegisterFlags(fs *flag.FlagSet, cmd string, c *config.Config) {
}

func (kc *Configurer) CheckFlags(fs *flag.FlagSet, c *config.Config) error {
	return nil
}
