package typescript

import (
	"fmt"
	"path"
	"path/filepath"
	"strings"
)

type TsConfigMap struct {
	configs map[string]*TsConfig
}

type TsWorkspace struct {
	cm *TsConfigMap
}

func NewTsWorkspace() *TsWorkspace {
	return &TsWorkspace{
		cm: &TsConfigMap{
			configs: make(map[string]*TsConfig),
		},
	}
}

func (tc *TsWorkspace) AddTsConfigFile(root, rel, fileName string) {
	_, err := parseTsConfigJSONFile(tc.cm, root, rel, fileName)
	if err != nil {
		fmt.Printf("Failed to parse tsconfig file %s: %v\n", path.Join(rel, fileName), err)
	}
}

func (tc *TsWorkspace) GetTsConfigFile(rel string) *TsConfig {
	c := tc.cm.configs[rel]
	if c == &InvalidTsconfig {
		return nil
	}
	return c
}

func (tc *TsWorkspace) getConfig(f string) (string, *TsConfig) {
	dir := f

	for dir = f; dir != ""; {
		dir = path.Dir(dir)
		if dir == "." {
			dir = ""
		}

		if c, exists := tc.cm.configs[dir]; exists && c != &InvalidTsconfig {
			return dir, c
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
