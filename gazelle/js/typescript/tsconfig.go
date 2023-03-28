package typescript

import (
	"os"
	"path"
	"path/filepath"
	"sort"
	"strings"

	"github.com/msolo/jsonr"
	"github.com/sirupsen/logrus"
)

type tsCompilerOptionsJSON struct {
	RootDir *string              `json:"rootDir"`
	BaseUrl *string              `json:"baseUrl"`
	Paths   *map[string][]string `json:"paths"`
}

type tsConfigJSON struct {
	Extends         string                `json:"extends"`
	CompilerOptions tsCompilerOptionsJSON `json:"compilerOptions"`
}

type TsConfig struct {
	ConfigDir string

	RootDir string
	BaseUrl string

	Paths *TsConfigPaths
}

type TsConfigPaths struct {
	Rel string
	Map *map[string][]string
}

var DefaultConfigPaths = TsConfigPaths{
	Rel: ".",
	Map: nil,
}

var Log = logrus.New()

// parseTsConfigJSONFile loads a tsconfig.json file and return the compilerOptions config
func parseTsConfigJSONFile(cm *TsConfigMap, root, dir, tsconfig string) (*TsConfig, error) {
	existing := cm.configs[dir]
	if existing != nil {
		return existing, nil
	}

	content, readErr := os.ReadFile(path.Join(root, dir, tsconfig))
	if readErr != nil {
		return nil, readErr
	}

	config, err := parseTsConfigJSON(cm, root, dir, content)
	cm.configs[dir] = config
	return config, err
}

func parseTsConfigJSON(cm *TsConfigMap, root, configDir string, tsconfigContent []byte) (*TsConfig, error) {
	var c tsConfigJSON
	if err := jsonr.Unmarshal(tsconfigContent, &c); err != nil {
		return nil, err
	}

	var baseConfig *TsConfig
	if c.Extends != "" {
		base, err := parseTsConfigJSONFile(cm, root, path.Join(configDir, path.Dir(c.Extends)), path.Base(c.Extends))
		if err != nil {
			Log.Warnf("Failed to load base tsconfig file %s: %v", path.Join(configDir, c.Extends), err)
		}

		baseConfig = base
	}

	var RootDir string
	if c.CompilerOptions.RootDir != nil {
		RootDir = path.Clean(*c.CompilerOptions.RootDir)
	} else {
		RootDir = "."
	}

	var BaseUrl string
	if c.CompilerOptions.BaseUrl != nil {
		BaseUrl = path.Clean(*c.CompilerOptions.BaseUrl)
	} else {
		BaseUrl = "."
	}

	var Paths *TsConfigPaths
	if c.CompilerOptions.Paths != nil {
		Paths = &TsConfigPaths{
			Rel: ".",
			Map: c.CompilerOptions.Paths,
		}
	} else if baseConfig != nil {
		rel, relErr := filepath.Rel(configDir, baseConfig.ConfigDir)
		if relErr != nil {
			Log.Warnf("Failed to resolve relative path from %s to %s: %v", configDir, baseConfig.ConfigDir, relErr)

			Paths = nil
		} else {
			Paths = &TsConfigPaths{
				Rel: path.Join(baseConfig.Paths.Rel, rel),
				Map: baseConfig.Paths.Map,
			}
		}
	} else {
		Paths = &DefaultConfigPaths
	}

	config := TsConfig{
		ConfigDir: configDir,
		RootDir:   RootDir,
		BaseUrl:   BaseUrl,
		Paths:     Paths,
	}

	return &config, nil
}

// Expand the given path to all possible mapped paths for this config, in priority order.
//
// Path matching algorithm based on ESBuild implementation
// Inspired by: https://github.com/evanw/esbuild/blob/deb93e92267a96575a6e434ff18421f4ef0605e4/internal/resolver/resolver.go#L1831-L1945
func (c TsConfig) ExpandPaths(from, p string) []string {
	pathMap := c.Paths.Map
	if pathMap == nil {
		return []string{}
	}

	possible := make([]string, 0)

	// Check for exact matches first
	exact := (*pathMap)[p]
	if exact != nil {
		for _, m := range exact {
			possible = append(possible, path.Clean(path.Join(c.Paths.Rel, m)))
		}
	}

	// Check for pattern matches next
	possibleMatches := make(matchArray, 0)
	for key, originalPaths := range *pathMap {
		if starIndex := strings.IndexByte(key, '*'); starIndex != -1 {
			prefix, suffix := key[:starIndex], key[starIndex+1:]

			if strings.HasPrefix(p, prefix) && strings.HasSuffix(p, suffix) {
				possibleMatches = append(possibleMatches, match{
					prefix:        prefix,
					suffix:        suffix,
					originalPaths: originalPaths,
				})
			}
		}
	}

	// Sort the pattern matches by priority
	sort.Sort(possibleMatches)

	// Expand and add the pattern matches
	for _, m := range possibleMatches {
		for _, originalPath := range m.originalPaths {
			// Swap out the "*" in the original path for whatever the "*" matched
			matchedText := p[len(m.prefix) : len(p)-len(m.suffix)]
			mappedPath := strings.Replace(originalPath, "*", matchedText, 1)

			mappedPath = path.Clean(mappedPath)

			possible = append(possible, path.Join(c.Paths.Rel, mappedPath))
		}
	}

	return possible
}

type match struct {
	prefix        string
	suffix        string
	originalPaths []string
}

type matchArray []match

func (s matchArray) Len() int {
	return len(s)
}
func (s matchArray) Swap(i, j int) {
	s[i], s[j] = s[j], s[i]
}

// Sort the same as TypeScript/ESBuild prioritize longer prefixes and suffixes
// See https://github.com/evanw/esbuild/blob/deb93e92267a96575a6e434ff18421f4ef0605e4/internal/resolver/resolver.go#L1895-L1901
func (s matchArray) Less(i, j int) bool {
	return len(s[i].prefix) > len(s[j].prefix) || len(s[i].prefix) == len(s[j].prefix) && len(s[i].suffix) > len(s[j].suffix)
}
