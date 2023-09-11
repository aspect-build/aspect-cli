/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package typescript

import (
	"os"
	"path"
	"path/filepath"
	"sort"
	"strings"

	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/msolo/jsonr"
)

type tsCompilerOptionsJSON struct {
	RootDir  *string              `json:"rootDir"`
	RootDirs *[]string            `json:"rootDirs"`
	BaseUrl  *string              `json:"baseUrl"`
	Paths    *map[string][]string `json:"paths"`
}

type tsConfigJSON struct {
	Extends         string                `json:"extends"`
	CompilerOptions tsCompilerOptionsJSON `json:"compilerOptions"`
}

type TsConfig struct {
	ConfigDir string

	RootDir string
	BaseUrl string

	VirtualRootDirs []string

	Paths *TsConfigPaths
}

type TsConfigPaths struct {
	Rel string
	Map *map[string][]string
}

var DefaultConfigPaths = TsConfigPaths{
	Rel: ".",
	Map: &map[string][]string{},
}

func isRelativePath(p string) bool {
	if path.IsAbs(p) {
		return false
	}

	return strings.HasPrefix(p, "./") || strings.HasPrefix(p, "../")
}

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
			BazelLog.Warnf("Failed to load base tsconfig file %s: %v", path.Join(configDir, c.Extends), err)
		}

		baseConfig = base
	}

	var baseConfigRel = "."
	if baseConfig != nil {
		rel, relErr := filepath.Rel(configDir, baseConfig.ConfigDir)
		if relErr != nil {
			BazelLog.Warnf("Failed to resolve relative path from %s to %s: %v", configDir, baseConfig.ConfigDir, relErr)
		} else {
			baseConfigRel = rel
		}
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
		Paths = &TsConfigPaths{
			Rel: path.Join(baseConfig.Paths.Rel, baseConfigRel),
			Map: baseConfig.Paths.Map,
		}
	} else {
		Paths = &DefaultConfigPaths
	}

	var VirtualRootDirs = make([]string, 0)
	if c.CompilerOptions.RootDirs != nil {
		for _, d := range *c.CompilerOptions.RootDirs {
			VirtualRootDirs = append(VirtualRootDirs, path.Clean(d))
		}
	} else if baseConfig != nil {
		for _, d := range baseConfig.VirtualRootDirs {
			VirtualRootDirs = append(VirtualRootDirs, path.Join(baseConfigRel, d))
		}
	}

	config := TsConfig{
		ConfigDir:       configDir,
		RootDir:         RootDir,
		BaseUrl:         BaseUrl,
		Paths:           Paths,
		VirtualRootDirs: VirtualRootDirs,
	}

	return &config, nil
}

// Returns the path from the project base to the active tsconfig.json file
// This is used to build the path from the project base to the file being imported
// because gazelle seems to resolve files relative to the project base
// if the passed path is not absolute.
// Or an empty string if the path is absolute
func (c TsConfig) expandRelativePath(importPath string) string {
	// Absolute paths must never be expanded but everything else must be relative to the bazel-root
	// and therefore expanded with the path to the current active tsconfig.json
	if !path.IsAbs(importPath) {
		BazelLog.Tracef("Found local path %s in tsconfig.json. Should be expanded with tsconfig dir: %s", importPath, c.ConfigDir)
		return c.ConfigDir
	}
	return ""
}

// Expand the given path to all possible mapped paths for this config, in priority order.
//
// Path matching algorithm based on ESBuild implementation
// Inspired by: https://github.com/evanw/esbuild/blob/deb93e92267a96575a6e434ff18421f4ef0605e4/internal/resolver/resolver.go#L1831-L1945
func (c TsConfig) ExpandPaths(from, p string) []string {
	pathMap := c.Paths.Map
	possible := make([]string, 0)

	// Check for exact 'paths' matches first
	exact := (*pathMap)[p]
	for _, m := range exact {
		possible = append(possible, path.Join(c.expandRelativePath(m), c.Paths.Rel, m))
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

	// Sort the 'paths' pattern matches by priority
	sort.Sort(possibleMatches)

	// Expand and add the pattern matches
	for _, m := range possibleMatches {
		for _, originalPath := range m.originalPaths {
			// Swap out the "*" in the original path for whatever the "*" matched
			matchedText := p[len(m.prefix) : len(p)-len(m.suffix)]
			mappedPath := strings.Replace(originalPath, "*", matchedText, 1)

			possible = append(possible, path.Join(c.expandRelativePath(mappedPath), c.Paths.Rel, mappedPath))
		}
	}

	// Expand paths from baseUrl
	// Must not to be absolute or relative to be expanded
	// https://www.typescriptlang.org/tsconfig#baseUrl
	if !isRelativePath(p) {
		possible = append(possible, path.Join(c.expandRelativePath(p), c.BaseUrl, p))
	}

	// Add 'rootDirs' as alternate directories for relative imports
	// https://www.typescriptlang.org/tsconfig#rootDirs
	for _, v := range c.VirtualRootDirs {
		possible = append(possible, path.Join(v, p))
	}

	BazelLog.Tracef("Found %d possible paths for %s: %v", len(possible), p, possible)

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
