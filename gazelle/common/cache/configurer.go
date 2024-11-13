package cache

import (
	"flag"
	"os"

	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/bazelbuild/bazel-gazelle/rule"
)

const gazelleExtensionKey = "__aspect:cache"

func NewConfigurer() config.Configurer {
	return &cacheConfigurer{}
}

// Fetch the shared cache for a given config
func Get[T any](config *config.Config) Cache {
	if v, ok := config.Exts[gazelleExtensionKey]; ok {
		return v.(Cache)
	}
	return nil
}

var _ config.Configurer = (*cacheConfigurer)(nil)
var _ language.FinishableLanguage = (*cacheConfigurer)(nil)

type cacheConfigurer struct {
	cache Cache
}

// Load + store the cache
func (cc *cacheConfigurer) RegisterFlags(fs *flag.FlagSet, cmd string, c *config.Config) {
	if diskCachePath := os.Getenv("ASPECT_CONFIGURE_CACHE"); diskCachePath != "" {
		cc.cache = NewDiskCache(diskCachePath)
	} else {
		cc.cache = Noop()
	}

	c.Exts[gazelleExtensionKey] = cc.cache
}

// Persist the cache
func (cc *cacheConfigurer) DoneGeneratingRules() {
	cc.cache.Persist()
}

func (*cacheConfigurer) CheckFlags(fs *flag.FlagSet, c *config.Config) error  { return nil }
func (*cacheConfigurer) KnownDirectives() []string                            { return nil }
func (*cacheConfigurer) Configure(c *config.Config, rel string, f *rule.File) {}
