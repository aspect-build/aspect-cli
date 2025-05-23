package cache

import "github.com/bazelbuild/bazel-gazelle/config"

type Cache interface {
	/** Load a key from the cache. */
	Load(key string) (any, bool)
	/** Store a key+value in the ache */
	Store(key string, value any)
	/** Store a key+value in the cache, or return the existing value */
	LoadOrStore(key string, value any) (any, bool)

	/** Persist any changes to the cache */
	Persist()

	/** Load+Store data computed from file contents.
	 *
	 * If the underlying file has changed since the data was computed, the
	 * loader should return false.
	 *
	 * The file content may or may not be read from disk, depending on the Cache
	 * implementation as well as the cache status.
	 *
	 * The path 'root' is not part of the cache key, but is used to resolve
	 * relative paths in the cache.
	 */
	LoadOrStoreFile(root, path, key string, loader FileCompute) (any, bool, error)
}

type FileCompute = func(path string, content []byte) (any, error)

type CacheFactory = func(c *config.Config) Cache
