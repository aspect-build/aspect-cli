package gazelle

import (
	"flag"
	"strconv"
	"sync"

	BazelLog "github.com/aspect-build/aspect-cli/gazelle/common/logger"
	"github.com/aspect-build/aspect-cli/gazelle/language/host/plugin"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"golang.org/x/sync/errgroup"
)

var _ config.Configurer = (*GazelleHost)(nil)

func (c *GazelleHost) KnownDirectives() []string {
	if c.gazelleDirectives == nil {
		c.gazelleDirectives = []string{}

		// TODO: verify no collisions with other plugins/globals

		for _, plugin := range c.plugins {
			// A directive to enable/disable the plugin
			c.gazelleDirectives = append(c.gazelleDirectives, plugin.Name())

			// Directives defined by the plugin
			for _, dir := range plugin.Properties() {
				c.gazelleDirectives = append(c.gazelleDirectives, dir.Name)
			}
		}
	}

	return c.gazelleDirectives
}

func (configurer *GazelleHost) Configure(c *config.Config, rel string, f *rule.File) {
	BazelLog.Tracef("Configure(%s): %s", GazelleLanguageName, rel)

	// Generate hierarchical configuration.
	if rel == "" {
		c.Exts[GazelleLanguageName] = NewRootConfig(c.RepoName)
	} else {
		c.Exts[GazelleLanguageName] = c.Exts[GazelleLanguageName].(*BUILDConfig).NewChildConfig(rel)
	}

	config := c.Exts[GazelleLanguageName].(*BUILDConfig)

	// Record directives from the existing BUILD file.
	if f != nil {
		for _, d := range f.Directives {
			config.appendDirectiveValue(d.Key, d.Value)
		}
	}

	eg := errgroup.Group{}
	eg.SetLimit(10)

	var prepResultMutex sync.Mutex

	// Prepare the plugins for this configuration.
	for k, p := range configurer.plugins {
		if !config.IsPluginEnabled(k) {
			continue
		}

		eg.Go(func() error {
			prepContext := configToPrepareContext(p, config)
			prepResult := p.Prepare(prepContext)

			// Lock while modifying config.pluginPrepareResults
			prepResultMutex.Lock()
			defer prepResultMutex.Unlock()

			// Index the plugins and their PrepareResult
			config.pluginPrepareResults[k] = pluginConfig{
				PrepareContext: prepContext,
				PrepareResult:  prepResult,
			}

			return nil
		})
	}

	if err := eg.Wait(); err != nil {
		BazelLog.Errorf("Configure(%s) plugin error: %v", GazelleLanguageName, err)
	}
}

func configToPrepareContext(p plugin.Plugin, cfg *BUILDConfig) plugin.PrepareContext {
	ctx := plugin.PrepareContext{
		RepoName:   cfg.repoName,
		Rel:        cfg.rel,
		Properties: plugin.NewPropertyValues(),
	}

	for k, p := range p.Properties() {
		pValue := p.Default

		if v, found := cfg.getRawValue(p.Name, true); found {
			parsedValue, parseErr := parsePropertyValue(p, v)
			if parseErr != nil {
				BazelLog.Warnf("Failed to parse property %q: %v", p.Name, parseErr)
			} else {
				pValue = parsedValue
			}
		}

		ctx.Properties.Add(k, pValue)
	}

	return ctx
}

func parsePropertyValue(p plugin.Property, values []string) (interface{}, error) {
	switch p.PropertyType {
	case plugin.PropertyType_String:
		return onlyValue(p, values), nil
	case plugin.PropertyType_Strings:
		return values, nil
	case plugin.PropertyType_Bool:
		return onlyValue(p, values) == "true", nil
	case plugin.PropertyType_Number:
		return strconv.ParseInt(onlyValue(p, values), 10, 0)
	}

	panic("unhandled property type: " + p.PropertyType)
}

func onlyValue(p plugin.Property, value []string) string {
	c := len(value)

	if c == 0 {
		BazelLog.Fatalf("expected exactly one value, got none")
		return ""
	} else if c > 1 {
		BazelLog.Warnf("expected exactly one value for %q, got %d", p.Name, c)
	}

	return value[c-1]
}

func (c *GazelleHost) RegisterFlags(fs *flag.FlagSet, cmd string, cfg *config.Config) {
}

func (c *GazelleHost) CheckFlags(fs *flag.FlagSet, cfg *config.Config) error {
	return nil
}
