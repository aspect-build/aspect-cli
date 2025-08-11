package gazelle

import (
	plugin "github.com/aspect-build/aspect-cli/gazelle/languages/host/plugin"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bmatcuk/doublestar/v4"
	"github.com/emirpasic/gods/maps/linkedhashmap"
)

type BUILDConfig struct {
	// Shared across all
	repoName string

	// This config
	rel    string
	parent *BUILDConfig

	// If this BUILD has been generated during this execution
	generated bool

	// All directives of this BUILD
	directiveRawValues map[string][]string

	// Custom/overridden resolutions
	resolves *linkedhashmap.Map

	// Plugin specific config
	pluginPrepareResults map[plugin.PluginId]pluginConfig
}

func NewRootConfig(repoName string) *BUILDConfig {
	return &BUILDConfig{
		repoName:           repoName,
		rel:                "",
		directiveRawValues: make(map[string][]string),

		resolves: linkedhashmap.New(),

		pluginPrepareResults: make(map[string]pluginConfig),
	}
}

func (c *BUILDConfig) NewChildConfig(rel string) *BUILDConfig {
	// TODO: freeze the parent config now that a child has copied/inherited it.

	cCopy := *c

	// Child specific
	cCopy.generated = false
	cCopy.rel = rel
	cCopy.parent = c
	cCopy.directiveRawValues = make(map[string][]string)

	// Non-inherited that require cloning
	// TODO: verify these should not be inherited
	cCopy.pluginPrepareResults = make(map[string]pluginConfig)

	// Inherited that must be cloned
	cCopy.resolves = linkedhashmap.New()
	c.resolves.Each(cCopy.resolves.Put)

	return &cCopy
}

func (p *BUILDConfig) appendDirectiveValue(key, value string) {
	values, valueExists := p.directiveRawValues[key]
	if !valueExists {
		p.directiveRawValues[key] = []string{value}
	} else {
		p.directiveRawValues[key] = append(values, value)
	}
}

func (c *BUILDConfig) IsPluginEnabled(pluginId plugin.PluginId) bool {
	if val, exists := c.getRawValue(string(pluginId), true); exists {
		return val[len(val)-1] == "enabled"
	}
	return true
}

func (c *BUILDConfig) getRawValue(key string, inherit bool) ([]string, bool) {
	value, exists := c.directiveRawValues[key]
	if exists {
		return value, true
	}

	if inherit && c.parent != nil {
		return c.parent.getRawValue(key, true)
	}

	return nil, false
}

func (c *BUILDConfig) GetResolution(imprt string) *label.Label {
	config := c
	for config != nil {
		for _, glob := range config.resolves.Keys() {
			if doublestar.MatchUnvalidated(glob.(string), imprt) {
				resolveLabel, _ := config.resolves.Get(glob)
				return resolveLabel.(*label.Label)
			}
		}
		config = config.parent
	}

	return nil
}

// An extension of PrepareContext+Result to add internal utils
type pluginConfig struct {
	plugin.PrepareContext
	plugin.PrepareResult
}

func (c *pluginConfig) GetQueriesForFile(f string) plugin.NamedQueries {
	fileQueries := make(plugin.NamedQueries)

	for n, query := range c.PrepareResult.Queries {
		if query.Match(f) {
			fileQueries[n] = query
			continue
		}
	}

	return fileQueries
}
