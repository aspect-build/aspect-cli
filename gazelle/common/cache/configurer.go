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
	return &cacheConfigurer{
		cache: noop,
	}
}

// Fetch the shared cache for a given config
func Get(config *config.Config) Cache {
	if v, ok := config.Exts[gazelleExtensionKey]; ok {
		return v.(Cache)
	}
	return noop
}

var cacheFactory CacheFactory

func init() {
	if diskCachePath := os.Getenv("ASPECT_CONFIGURE_CACHE"); diskCachePath != "" {
		cacheFactory = func(c *config.Config) Cache {
			return NewDiskCache(diskCachePath)
		}
	}
}

var _ config.Configurer = (*cacheConfigurer)(nil)
var _ language.FinishableLanguage = (*cacheConfigurer)(nil)

type cacheConfigurer struct {
	cache Cache
}

func SetCacheFactory(c CacheFactory) {
	cacheFactory = c
}

// Load + store the cache
func (cc *cacheConfigurer) CheckFlags(fs *flag.FlagSet, c *config.Config) error {
	if cacheFactory == nil {
		cc.cache = noop
	} else {
		cc.cache = cacheFactory(c)
	}
	c.Exts[gazelleExtensionKey] = cc.cache
	return nil
}

// Persist the cache
func (cc *cacheConfigurer) DoneGeneratingRules() {
	cc.cache.Persist()
}

func (cc *cacheConfigurer) RegisterFlags(fs *flag.FlagSet, cmd string, c *config.Config) {}
func (*cacheConfigurer) KnownDirectives() []string                                       { return nil }
func (*cacheConfigurer) Configure(c *config.Config, rel string, f *rule.File)            {}
