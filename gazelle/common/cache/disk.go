package cache

import (
	"crypto"
	"encoding/gob"
	"encoding/hex"
	"os"
	"path"
	"sync"

	"github.com/aspect-build/aspect-cli/buildinfo"
	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
)

func NewDiskCache(cacheFilePath string) Cache {
	c := &diskCache{
		file: cacheFilePath,
		old:  map[string]any{},
		new:  &sync.Map{},
	}
	c.read()
	return c
}

func init() {
	// Register some basic types for gob serialization so languages
	// only have to register custom types.
	gob.Register(map[string]interface{}{})
	gob.Register(map[string]string{})
	gob.Register([]interface{}{})
}

var _ Cache = (*diskCache)(nil)

type diskCache struct {
	file string
	old  map[string]any
	new  *sync.Map
}

func computeCacheKey(content []byte) (string, bool) {
	cacheDigest := crypto.MD5.New()

	if buildinfo.IsStamped() {
		if _, err := cacheDigest.Write([]byte(buildinfo.GitCommit)); err != nil {
			BazelLog.Errorf("Failed to write GitCommit to cache digest: %v", err)
			return "", false
		}
	}

	if _, err := cacheDigest.Write(content); err != nil {
		BazelLog.Errorf("Failed to write source to cache digest: %v", err)
		return "", false
	}

	return hex.EncodeToString(cacheDigest.Sum(nil)), true
}

func (c *diskCache) read() {
	cacheReader, err := os.Open(c.file)
	if err != nil {
		BazelLog.Tracef("Failed to open cache %q: %v", c.file, err)
		return
	}
	defer cacheReader.Close()

	cacheDecoder := gob.NewDecoder(cacheReader)
	if e := cacheDecoder.Decode(&c.old); e != nil {
		BazelLog.Errorf("Failed to read cache %q: %v", c.file, e)
	}
}

func (c *diskCache) write() {
	cacheWriter, err := os.OpenFile(c.file, os.O_RDWR|os.O_CREATE, 0666)
	if err != nil {
		BazelLog.Errorf("Failed to create cache %q: %v", c.file, err)
		return
	}
	defer cacheWriter.Close()

	m := make(map[string]any)
	c.new.Range(func(key, value interface{}) bool {
		m[key.(string)] = value
		return true
	})

	cacheEncoder := gob.NewEncoder(cacheWriter)
	if e := cacheEncoder.Encode(m); e != nil {
		BazelLog.Errorf("Failed to write cache %q: %v", c.file, e)
	}
}

func (c *diskCache) Load(key string) (any, bool) {
	if v, found := c.new.Load(key); found {
		return v, true
	}

	if v, ok := c.old[key]; ok {
		c.new.LoadOrStore(key, v)
		return v, true
	}

	return nil, false
}

func (c *diskCache) Store(key string, value any) {
	c.new.Store(key, value)
}

func (c *diskCache) LoadAndStoreFile(root, p string, loader func(path string, content []byte) (any, error)) (any, bool, error) {
	content, err := os.ReadFile(path.Join(root, p))
	if err != nil {
		return nil, false, err
	}

	if parserCacheKey, parsingCacheable := computeCacheKey(content); parsingCacheable {
		// Try loading from the cache.
		if v, found := c.Load(parserCacheKey); found {
			return v, true, nil
		}

		// Compute and persist in cache.
		v, err := loader(p, content)
		if err != nil {
			c.Store(parserCacheKey, v)
		}

		return v, false, err
	}

	// Not cacheable, simply recompute each time.
	v, err := loader(p, content)
	return v, false, err
}

func (c *diskCache) Persist() {
	c.write()
}
