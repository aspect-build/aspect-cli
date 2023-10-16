package typescript

import (
	"fmt"
	"path"
	"path/filepath"
	"strings"
)

type workspacePath struct {
	root     string
	rel      string
	fileName string
}

type TsConfigMap struct {
	configFiles map[string]*workspacePath

	configs map[string]*TsConfig
}

type TsWorkspace struct {
	cm *TsConfigMap
}

func NewTsWorkspace() *TsWorkspace {
	return &TsWorkspace{
		cm: &TsConfigMap{
			configFiles: make(map[string]*workspacePath),
			configs:     make(map[string]*TsConfig),
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
	// Previously parsed
	if c := tc.cm.configs[rel]; c != nil {
		if c == &InvalidTsconfig {
			return nil
		}
		return c
	}

	// Does not exist
	p := tc.cm.configFiles[rel]
	if p == nil {
		return nil
	}

	c, err := parseTsConfigJSONFile(tc.cm, p.root, p.rel, p.fileName)
	if err != nil {
		fmt.Printf("Failed to parse tsconfig file %s: %v\n", path.Join(p.rel, p.fileName), err)
		return nil
	}

	return c
}

func (tc *TsWorkspace) hasConfig(rel string) bool {
	return tc.cm.configFiles[rel] != nil && tc.cm.configs[rel] != &InvalidTsconfig
}

func (tc *TsWorkspace) getConfig(f string) (string, *TsConfig) {
	dir := f

	for dir = f; dir != ""; {
		dir = path.Dir(dir)
		if dir == "." {
			dir = ""
		}

		if tc.hasConfig(dir) {
			return dir, tc.GetTsConfigFile(dir)
		}
	}

	return "", nil
}

func (tc *TsWorkspace) IsWithinTsRoot(f string) bool {
	dir, c := tc.getConfig(f)
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
	_, c := tc.getConfig(from)
	if c == nil {
		return []string{}
	}

	return c.ExpandPaths(from, f)
}
