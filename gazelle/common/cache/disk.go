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

/**
 * Cache to disk, keyed by file content hash and `buildinfo.GitCommit`.
 */
func NewDiskCache(cacheFilePath string) Cache {
	c := &diskCache{
		file: cacheFilePath,
		old:  map[string]map[string]any{},
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
	gob.Register(map[string]map[string]interface{}{})
	gob.Register(map[string]map[string]string{})
	gob.Register([]interface{}{})
}

var _ Cache = (*diskCache)(nil)

type diskCache struct {
	// Where the cache is persisted to disk.
	file string

	// A cache mapped by file-content-hash => map[key]value
	old map[string]map[string]any

	// A cache mapped by file path => cacheEntry
	new *sync.Map
}

type cacheEntry struct {
	contentHash string
	values      *sync.Map
}

func computeCacheKey(content []byte) string {
	cacheDigest := crypto.MD5.New()
	if buildinfo.IsStamped() {
		cacheDigest.Write([]byte(buildinfo.GitCommit))
	}
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

	m := make(map[string]map[string]any)
	c.new.Range(func(p, e interface{}) bool {
		ce := e.(*cacheEntry)
		ce.values.Range(func(k, v interface{}) bool {
			if _, ok := m[ce.contentHash]; !ok {
				m[ce.contentHash] = make(map[string]any)
			}
			m[ce.contentHash][k.(string)] = v
			return true
		})
		return true
	})

	cacheEncoder := gob.NewEncoder(cacheWriter)
	if e := cacheEncoder.Encode(m); e != nil {
		BazelLog.Errorf("Failed to write cache %q: %v", c.file, e)
		return
	}

	BazelLog.Infof("Wrote %d entries to cache %q\n", len(m), c.file)
}

func (c *diskCache) LoadOrStoreFile(root, p, key string, loader FileCompute) (any, bool, error) {
	content, err := os.ReadFile(path.Join(root, p))
	if err != nil {
		return nil, false, err
	}

	// Include the file content in the cache key
	contentKey := computeCacheKey(content)

	pCache, pCacheFound := c.new.Load(p)

	// Try loading from the cache if exists and content has not changed.
	if pCacheFound && pCache.(*cacheEntry).contentHash == contentKey {
		if v, found := pCache.(*cacheEntry).values.Load(key); found {
			return v, true, nil
		}
	} else {
		pCache = &cacheEntry{
			contentHash: contentKey,
			values:      &sync.Map{},
		}
		c.new.LoadOrStore(p, pCache)
	}

	// Try loading from the old cache and populate the new.
	if oldCache, found := c.old[contentKey]; found {
		if v, found := oldCache[key]; found {
			v, _ := pCache.(*cacheEntry).values.LoadOrStore(key, v)
			return v, true, nil
		}
	}

	// Compute and persist the value.
	v, err := loader(p, content)
	if err != nil {
		return nil, false, err
	}

	v, found := pCache.(*cacheEntry).values.LoadOrStore(key, v)
	return v, found, nil
}

func (c *diskCache) Persist() {
	c.write()
}
