package kotlinconfig

import (
	"path/filepath"

	"github.com/bazel-contrib/rules_jvm/java/gazelle/javaconfig"
)

type KotlinConfig struct {
	*javaconfig.Config

	parent *KotlinConfig
	rel    string
}

type Configs = map[string]*KotlinConfig

func New(repoRoot string) *KotlinConfig {
	return &KotlinConfig{
		Config: javaconfig.New(repoRoot),
		parent: nil,
	}
}

func (c *KotlinConfig) NewChild(childPath string) *KotlinConfig {
	cCopy := *c
	cCopy.Config = c.Config.NewChild()
	cCopy.rel = childPath
	cCopy.parent = c
	return &cCopy
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
