package typescript

import (
	"fmt"
	"path"
	"sync"

	node "aspect.build/cli/gazelle/js/node"
	pnpm "aspect.build/cli/gazelle/js/pnpm"
	BazelLog "aspect.build/cli/pkg/logger"
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
	pnpmProjects *pnpm.PnpmProjectMap
}

type TsWorkspace struct {
	cm *TsConfigMap
}

func NewTsWorkspace(pnpmProjects *pnpm.PnpmProjectMap) *TsWorkspace {
	return &TsWorkspace{
		cm: &TsConfigMap{
			configFiles:  make(map[string]*workspacePath),
			configs:      make(map[string]*TsConfig),
			pnpmProjects: pnpmProjects,
			configsMutex: sync.RWMutex{},
		},
	}
}

func (tc *TsWorkspace) AddTsConfigFile(root, rel, fileName string) {
	if c := tc.cm.configFiles[rel]; c != nil {
		fmt.Printf("Duplicate tsconfig file %s: %s and %s", path.Join(rel, fileName), c.rel, c.fileName)
		return
	}

	BazelLog.Debugf("Adding tsconfig file %s/%s", rel, fileName)

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

	c, err := parseTsConfigJSONFile(tc.cm.configs, tc.tsConfigResolver, p.root, path.Join(p.rel, p.fileName))
	if err != nil {
		fmt.Printf("Failed to parse tsconfig file %s: %v\n", path.Join(p.rel, p.fileName), err)
		return nil
	}

	BazelLog.Debugf("Parsed tsconfig file %s/%s", p.rel, p.fileName)

	return c
}

// A `TsConfigResolver` to resolve imports from *within* tsconfig files
// to real paths such as resolved the tsconfig `extends`.
func (tc *TsWorkspace) tsConfigResolver(dir, rel string) []string {
	possible := []string{}

	if isRelativePath(rel) {
		possible = append(possible, path.Join(dir, rel))
	}

	if p := tc.cm.pnpmProjects.GetProject(dir); p != nil {
		pkg, subFile := node.ParseImportPath(rel)
		if pkg != "" {
			localRef, found := p.GetLocalReference(pkg)
			if found {
				possible = append(possible, path.Join(localRef, subFile))
			}
		}
	}

	return possible
}

func (tc *TsWorkspace) FindConfig(dir string) (string, *TsConfig) {
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

func (tc *TsWorkspace) ExpandPaths(from, f string) []string {
	_, c := tc.FindConfig(path.Dir(from))
	if c == nil {
		return []string{}
	}

	return c.ExpandPaths(from, f)
}
