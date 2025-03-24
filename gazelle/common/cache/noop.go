package cache

import (
	"os"
	"path"
)

func Noop() Cache {
	return &noopCache{}
}

var _ Cache = (*noopCache)(nil)

type noopCache struct{}

func (c *noopCache) Load(key string) (any, bool) { return nil, false }
func (c *noopCache) Store(key string, value any) {}
func (c *noopCache) LoadAndStoreFile(root, p string, loader func(p string, content []byte) (any, error)) (any, bool, error) {
	content, err := os.ReadFile(path.Join(root, p))
	if err != nil {
		return nil, false, err
	}

	result, err := loader(p, content)
	return result, false, err
}

func (c *noopCache) Persist() {}
