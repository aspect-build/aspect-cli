package cache

func Noop() Cache {
	return &noopCache{}
}

var _ Cache = (*noopCache)(nil)

type noopCache struct{}

func (c *noopCache) Load(key string) (any, bool) { return nil, false }
func (c *noopCache) Store(key string, value any) {}
func (c *noopCache) Persist()                    {}
