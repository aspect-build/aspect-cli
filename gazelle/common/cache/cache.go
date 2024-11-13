package cache

type Cache interface {
	Load(key string) (any, bool)
	Store(key string, value any)
	Persist()
}
