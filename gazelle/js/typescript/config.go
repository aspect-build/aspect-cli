package typescript

import (
	"fmt"
	"path"
	"path/filepath"
	"strings"
	"sync"
)

type workspacePath struct {
	root     string
	rel      string
	fileName string
}

type TsConfigMap struct {
	// `configFiles` is created during the gazelle configure phase which is single threaded so doesn't
	// require mutex projection. Just `configs` has concurrency considerations since it is lazy
	// loading on multiple threads in the generate phase.
	configFiles  map[string]*workspacePath
	configs      map[string]*TsConfig
	configsMutex sync.RWMutex
}

type TsWorkspace struct {
	cm *TsConfigMap
}

func NewTsWorkspace() *TsWorkspace {
	return &TsWorkspace{
		cm: &TsConfigMap{
			configFiles:  make(map[string]*workspacePath),
			configs:      make(map[string]*TsConfig),
			configsMutex: sync.RWMutex{},
		},
	}
}

func (tc *TsWorkspace) AddTsConfigFile(root, rel, fileName string) {
	if c := tc.cm.configFiles[rel]; c != nil {
		fmt.Printf("Duplicate tsconfig file %s: %s and %s", path.Join(rel, fileName), c.rel, c.fileName)
		return
	}

	tc.cm.configFiles[rel] = &workspacePath{
		root:     root,
		rel:      rel,
		fileName: fileName,
	}
}

func (tc *TsWorkspace) GetTsConfigFile(rel string) *TsConfig {
	// No file exists
	p := tc.cm.configFiles[rel]
	if p == nil {
		return nil
	}

	// Lock the configs mutex
	tc.cm.configsMutex.Lock()
	defer tc.cm.configsMutex.Unlock()

	// Check for previously parsed
	if c := tc.cm.configs[rel]; c != nil {
		if c == &InvalidTsconfig {
			return nil
		}
		return c
	}

	c, err := parseTsConfigJSONFile(tc.cm.configs, p.root, p.rel, p.fileName)
	if err != nil {
		fmt.Printf("Failed to parse tsconfig file %s: %v\n", path.Join(p.rel, p.fileName), err)
		return nil
	}

	return c
}

func (tc *TsWorkspace) ResolveConfig(dir string) (string, *TsConfig) {
	for {
		if dir == "." {
			dir = ""
		}

		if tc.cm.configFiles[dir] != nil {
			return dir, tc.GetTsConfigFile(dir)
		}

		if dir == "" {
			break
		}

		dir = path.Dir(dir)
	}

	return "", nil
}

func (tc *TsWorkspace) IsWithinTsRoot(f string) bool {
	dir, c := tc.ResolveConfig(path.Dir(f))
	if c == nil {
		return true
	}

	if c.RootDir == "" {
		return true
	}

	rootRelative, relErr := filepath.Rel(path.Join(dir, c.RootDir), f)

	return relErr == nil && !strings.Contains(rootRelative, "..")
}

func (tc *TsWorkspace) ExpandPaths(from, f string) []string {
	_, c := tc.ResolveConfig(path.Dir(from))
	if c == nil {
		return []string{}
	}

	return c.ExpandPaths(from, f)
}
