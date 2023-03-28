package typescript

import (
	"os"
	"path"
	"sort"
	"strings"

	"github.com/msolo/jsonr"
)

type tsCompilerOptionsJSON struct {
	RootDir string              `json:"rootDir"`
	BaseUrl string              `json:"baseUrl"`
	Paths   map[string][]string `json:"paths"`
}

type tsConfigJSON struct {
	CompilerOptions tsCompilerOptionsJSON `json:"compilerOptions"`
}

type TsConfig struct {
	RootDir string
	BaseUrl string

	Paths map[string][]string
}

// parseTsConfigJSONFile loads a tsconfig.json file and return the compilerOptions config
func parseTsConfigJSONFile(root, dir, tsconfig string) (*TsConfig, error) {
	content, readErr := os.ReadFile(path.Join(root, dir, tsconfig))
	if readErr != nil {
		return nil, readErr
	}

	return parseTsConfigJSON(dir, content)
}

func parseTsConfigJSON(configDir string, tsconfigJSON []byte) (*TsConfig, error) {
	var c tsConfigJSON
	if err := jsonr.Unmarshal(tsconfigJSON, &c); err != nil {
		return nil, err
	}

	// TODO: extends

	RootDir := path.Clean(c.CompilerOptions.RootDir)
	BaseUrl := path.Clean(c.CompilerOptions.BaseUrl)

	config := TsConfig{
		RootDir: RootDir,
		BaseUrl: BaseUrl,

		Paths: c.CompilerOptions.Paths,
	}

	return &config, nil
}

// Expand the given path to all possible mapped paths for this config, in priority order.
//
// Path matching algorithm based on ESBuild implementation
// Inspired by: https://github.com/evanw/esbuild/blob/deb93e92267a96575a6e434ff18421f4ef0605e4/internal/resolver/resolver.go#L1831-L1945
func (c TsConfig) ExpandPaths(p string) []string {
	possible := make([]string, 0, 1)

	possible = append(possible, p)

	// Check for exact matches first
	exact := c.Paths[p]
	if exact != nil {
		for _, m := range exact {
			possible = append(possible, path.Clean(m))
		}
	}

	// Check for pattern matches next
	possibleMatches := make(matchArray, 0)
	for key, originalPaths := range c.Paths {
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

			possible = append(possible, mappedPath)
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
