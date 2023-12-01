package kotlinconfig

import (
	"path/filepath"

	"github.com/bazel-contrib/rules_jvm/java/gazelle/javaconfig"
)

const Directive_KotlinExtension = "kotlin"

type KotlinConfig struct {
	*javaconfig.Config

	parent *KotlinConfig
	rel    string

	generationEnabled bool
}

type Configs = map[string]*KotlinConfig

func New(repoRoot string) *KotlinConfig {
	return &KotlinConfig{
		Config:            javaconfig.New(repoRoot),
		generationEnabled: true,
		parent:            nil,
	}
}

func (c *KotlinConfig) NewChild(childPath string) *KotlinConfig {
	cCopy := *c
	cCopy.Config = c.Config.NewChild()
	cCopy.rel = childPath
	cCopy.parent = c
	return &cCopy
}

// SetGenerationEnabled sets whether the extension is enabled or not.
func (c *KotlinConfig) SetGenerationEnabled(enabled bool) {
	c.generationEnabled = enabled
}

// GenerationEnabled returns whether the extension is enabled or not.
func (c *KotlinConfig) GenerationEnabled() bool {
	return c.generationEnabled
}

// ParentForPackage returns the parent Config for the given Bazel package.
func ParentForPackage(c Configs, pkg string) *KotlinConfig {
	dir := filepath.Dir(pkg)
	if dir == "." {
		dir = ""
	}
	parent := (map[string]*KotlinConfig)(c)[dir]
	return parent
}
