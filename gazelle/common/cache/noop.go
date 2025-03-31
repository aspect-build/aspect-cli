package cache

import (
	"os"
	"path"
)

var _ Cache = (*noopCache)(nil)

var noop Cache = &noopCache{}

type noopCache struct{}

func (c *noopCache) Load(key string) (any, bool) { return nil, false }
func (c *noopCache) Store(key string, value any) {}
func (c *noopCache) LoadOrStore(key string, value any) (any, bool) {
	return value, false
}
func (c *noopCache) LoadOrStoreFile(root, p, key string, loader FileCompute) (any, bool, error) {
	content, err := os.ReadFile(path.Join(root, p))
	if err != nil {
		return nil, false, err
	}

	result, err := loader(p, content)
	return result, false, err
}

func (c *noopCache) Persist() {}
