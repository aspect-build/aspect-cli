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
	"fmt"
	"os"
	"path"
	"path/filepath"
	"sort"
	"strings"

	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/msolo/jsonr"
)

type tsCompilerOptionsJSON struct {
	OutDir   *string              `json:"outDir"`
	RootDir  *string              `json:"rootDir"`
	RootDirs *[]string            `json:"rootDirs"`
	BaseUrl  *string              `json:"baseUrl"`
	Paths    *map[string][]string `json:"paths"`
	Types    *[]string            `json:"types"`
}

type tsReferenceJSON struct {
	Path string `json:"path"`
}

type tsConfigJSON struct {
	Extends         string                `json:"extends"`
	CompilerOptions tsCompilerOptionsJSON `json:"compilerOptions"`
	References      *[]tsReferenceJSON    `json:"references"`
}

type TsConfigResolver = func(dir, conf string) []string

type TsConfig struct {
	// Directory of the tsconfig file
	ConfigDir string

	// Name of the tsconfig file relative to ConfigDir
	ConfigName string

	OutDir  string
	RootDir string
	BaseUrl string

	VirtualRootDirs []string

	Paths *TsConfigPaths

	// References to other tsconfig or packages that must be resolved.
	Types   []string
	Extends string

	// TODO: drop references? Not supported by rules_ts?
	References []string
}

type TsConfigPaths struct {
	Rel string
	Map *map[string][]string
}

var DefaultConfigPaths = TsConfigPaths{
	Rel: ".",
	Map: &map[string][]string{},
}

var InvalidTsconfig = TsConfig{
	Paths: &DefaultConfigPaths,
}

func isRelativePath(p string) bool {
	if path.IsAbs(p) {
		return false
	}

	return strings.HasPrefix(p, "./") || strings.HasPrefix(p, "../")
}

// Load a tsconfig.json file and return the compilerOptions config with
// recursive protected via a parsed map that is passed in
func parseTsConfigJSONFile(parsed map[string]*TsConfig, resolver TsConfigResolver, root, dir, tsconfig string) (*TsConfig, error) {
	key := path.Join(dir, tsconfig)

	existing := parsed[key]

	// Existing pointing to `InvalidTsconfig` implies recursion
	if existing == &InvalidTsconfig {
		BazelLog.Warnf("Recursive tsconfig file extension: %q", tsconfig)
		return nil, nil
	}

	// Already parsed and cached
	if existing != nil {
		return existing, nil
	}

	// Start with invalid to prevent recursing into the same file
	parsed[key] = &InvalidTsconfig

	potentialConfigs := resolver(dir, tsconfig)

	for _, potentialTsConfig := range potentialConfigs {
		content, err := os.ReadFile(path.Join(root, potentialTsConfig))
		if err != nil {
			BazelLog.Tracef("TsConfig lookup of %q from %q as %q not found: %v", tsconfig, dir, potentialTsConfig, err)
			continue
		}

		config, err := parseTsConfigJSON(parsed, resolver, root, dir, tsconfig, content)
		if err != nil {
			return nil, err
		}

		// Add to parsed map on success
		parsed[key] = config
		return config, nil
	}

	return nil, fmt.Errorf("tsconfig %q from %q not found", tsconfig, dir)
}

func parseTsConfigJSON(parsed map[string]*TsConfig, resolver TsConfigResolver, root, configDir, configName string, tsconfigContent []byte) (*TsConfig, error) {
	var c tsConfigJSON
	if err := jsonr.Unmarshal(tsconfigContent, &c); err != nil {
		return nil, err
	}

	var baseConfig *TsConfig
	var extends string
	if c.Extends != "" {
		base, err := parseTsConfigJSONFile(parsed, resolver, root, configDir, c.Extends)
		if err != nil {
			BazelLog.Warnf("Failed to load base tsconfig file %q from %q: %v", c.Extends, path.Join(configDir, configName), err)
		} else if base != nil {
			extends = path.Clean(c.Extends)
			baseConfig = base
		}
	}

	var types []string
	if c.CompilerOptions.Types != nil && len(*c.CompilerOptions.Types) > 0 {
		types = *c.CompilerOptions.Types
	}

	var references []string
	if c.References != nil && len(*c.References) > 0 {
		references = make([]string, 0, len(*c.References))

		for _, r := range *c.References {
			if r.Path != "" {
				references = append(references, path.Join(configDir, r.Path))
			}
		}
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

	var OutDir string
	if c.CompilerOptions.OutDir != nil {
		OutDir = path.Clean(*c.CompilerOptions.OutDir)
	} else {
		OutDir = "."
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
			Rel: BaseUrl,
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
		ConfigName:      configName,
		OutDir:          OutDir,
		RootDir:         RootDir,
		BaseUrl:         BaseUrl,
		Paths:           Paths,
		VirtualRootDirs: VirtualRootDirs,
		Extends:         extends,
		Types:           types,
		References:      references,
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
	if exact := (*pathMap)[p]; len(exact) > 0 {
		BazelLog.Tracef("TsConfig.paths exact matches for %q: %v", p, exact)

		for _, m := range exact {
			possible = append(possible, path.Join(c.expandRelativePath(m), c.Paths.Rel, m))
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

	if len(possibleMatches) > 0 {
		// Sort the 'paths' pattern matches by priority
		sort.Sort(possibleMatches)

		BazelLog.Tracef("TsConfig.paths glob matches for %q: %v", p, possibleMatches)

		// Expand and add the pattern matches
		for _, m := range possibleMatches {
			for _, originalPath := range m.originalPaths {
				// Swap out the "*" in the original path for whatever the "*" matched
				matchedText := p[len(m.prefix) : len(p)-len(m.suffix)]
				mappedPath := strings.Replace(originalPath, "*", matchedText, 1)

				possible = append(possible, path.Join(c.expandRelativePath(mappedPath), c.Paths.Rel, mappedPath))
			}
		}
	}

	// Expand paths from baseUrl
	// Must not to be absolute or relative to be expanded
	// https://www.typescriptlang.org/tsconfig#baseUrl
	if !isRelativePath(p) {
		baseUrlPath := path.Join(c.expandRelativePath(p), c.BaseUrl, p)

		BazelLog.Tracef("TsConfig.baseUrl match for %q: %v", p, baseUrlPath)

		possible = append(possible, baseUrlPath)
	}

	// Add 'rootDirs' as alternate directories for relative imports
	// https://www.typescriptlang.org/tsconfig#rootDirs
	for _, v := range c.VirtualRootDirs {
		possible = append(possible, path.Join(v, p))
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
