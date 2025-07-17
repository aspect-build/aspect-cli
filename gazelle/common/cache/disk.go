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
	gob.Register(map[string][]string{})
	gob.Register(map[string]map[string]string{})
	gob.Register([]interface{}{})
}

var _ Cache = (*diskCache)(nil)

type diskCache struct {
	file string
	old  map[string]any
	new  *sync.Map
}

func computeCacheKey(content []byte, key string) string {
	cacheDigest := crypto.MD5.New()
	if buildinfo.IsStamped() {
		cacheDigest.Write([]byte(buildinfo.GitCommit))
	}
	cacheDigest.Write([]byte(key))
	return hex.EncodeToString(cacheDigest.Sum(content))
}

func (c *diskCache) read() {
	cacheReader, err := os.Open(c.file)
	if err != nil {
		BazelLog.Infof("Failed to open cache %q: %v", c.file, err)
		return
	}
	defer cacheReader.Close()

	cacheDecoder := gob.NewDecoder(cacheReader)
	if e := cacheDecoder.Decode(&c.old); e != nil {
		BazelLog.Errorf("Failed to read cache %q: %v", c.file, e)
		return
	}

	BazelLog.Infof("Loaded %d entries from cache %q\n", len(c.old), c.file)
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
		return
	}

	BazelLog.Infof("Wrote %d entries to cache %q\n", len(m), c.file)
}

func (c *diskCache) Load(key string) (any, bool) {
	// Already written to new cache.
	if v, found := c.new.Load(key); found {
		return v, true
	}

	// Exists in old cache and can transfer to new.
	if v, ok := c.old[key]; ok {
		v, _ = c.LoadOrStore(key, v)
		return v, true
	}

	// Cache miss
	return nil, false
}

func (c *diskCache) Store(key string, value any) {
	c.new.Store(key, value)
}

func (c *diskCache) LoadOrStore(key string, value any) (any, bool) {
	return c.new.LoadOrStore(key, value)
}

func (c *diskCache) LoadOrStoreFile(root, p, key string, loader FileCompute) (any, bool, error) {
	content, err := os.ReadFile(path.Join(root, p))
	if err != nil {
		return nil, false, err
	}

	// Include the file content in the cache key
	key = computeCacheKey(content, key)

	// Try loading from the cache.
	v, found := c.Load(key)

	// Compute and persist in cache.
	if !found {
		v, err = loader(p, content)
		if err == nil {
			v, found = c.LoadOrStore(key, v)
		}
	}

	return v, found, err
}

func (c *diskCache) Persist() {
	c.write()
}
