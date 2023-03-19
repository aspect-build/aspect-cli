package typescript

import (
	"fmt"
	"path"
	"path/filepath"
	"strings"
)

// TODO(jbedard): rootDirs, baseUrl, paths etc to resolve imports
// TODO(jbedard): support multi-file configs (extends)

type TsConfigMap struct {
	configs map[string]*TsConfigJSON
}

type TsWorkspace struct {
	cm *TsConfigMap
}

func NewTsWorkspace() *TsWorkspace {
	return &TsWorkspace{
		cm: &TsConfigMap{
			configs: make(map[string]*TsConfigJSON),
		},
	}
}

func (tc *TsWorkspace) AddTsConfigFile(root, rel, fileName string) error {
	tsconfigJSON, err := parseTsConfigJSONFile(path.Join(root, rel, fileName))
	if err != nil {
		fmt.Printf("Failed to parse tsconfig file %s: %v\n", path.Join(rel, fileName), err)
		return err
	}

	tc.cm.configs[rel] = tsconfigJSON
	return nil
}

func (tc *TsWorkspace) getConfig(f string) (string, *TsConfigJSON) {
	dir := f

	for dir = f; dir != ""; {
		dir = path.Dir(dir)
		if dir == "." {
			dir = ""
		}

		if c, exists := tc.cm.configs[dir]; exists {
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

	if c.CompilerOptions.RootDir == "" {
		return true
	}

	rootRelative, relErr := filepath.Rel(path.Join(dir, c.CompilerOptions.RootDir), f)

	return relErr == nil && !strings.Contains(rootRelative, "..")
}
