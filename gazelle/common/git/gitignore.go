package git

import (
	"bufio"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path"
	"strings"

	common "github.com/aspect-build/aspect-cli/gazelle/common"
	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/rule"
	gitignore "github.com/go-git/go-git/v5/plumbing/format/gitignore"
)

// TODO: remove and align with gazelle after https://github.com/aspect-build/aspect-cli/issues/755

// Must align with patched bazel-gazelle
const ASPECT_GITIGNORE = "__aspect:gitignore"

type isGitIgnoredFunc = func(string, bool) bool

// Directive to enable/disable gitignore support
const Directive_GitIgnore = "gitignore"

// Internal
const enabledExt = Directive_GitIgnore
const lastConfiguredExt = "gitignore:dir"
const ignorePatternsExt = "gitignore:patterns"

func collectIgnoreFiles(c *config.Config, rel string) {
	// Only parse once per directory.
	if lastCollected, hasCollected := c.Exts[lastConfiguredExt]; hasCollected && lastCollected == rel {
		return
	}
	c.Exts[lastConfiguredExt] = rel

	ents := c.Exts[common.ASPECT_DIR_ENTRIES].(map[string]fs.DirEntry)
	if _, hasIgnore := ents[".gitignore"]; !hasIgnore {
		return
	}

	// Find and add .gitignore files from this directory
	ignoreFilePath := path.Join(c.RepoRoot, rel, ".gitignore")
	ignoreReader, ignoreErr := os.Open(ignoreFilePath)
	if ignoreErr == nil {
		BazelLog.Tracef("Add ignore file %s/.gitignore", rel)
		defer ignoreReader.Close()
		addIgnore(c, rel, ignoreReader)
	} else {
		msg := fmt.Sprintf("Failed to open %s/.gitignore: %v", rel, ignoreErr)
		BazelLog.Error(msg)
		fmt.Printf("%s\n", msg)
	}
}

func ReadGitConfig(c *config.Config, rel string, f *rule.File) {
	// Enable .gitignore support by default in Aspect gazelle languages.
	// TODO: default to false and encourage use of .bazelignore instead
	if rel == "" {
		_, exists := c.Exts[enabledExt]
		if !exists {
			c.Exts[enabledExt] = true
		}
	}

	// Collect ignore files within this config directory.
	collectIgnoreFiles(c, rel)

	// Collect config from directives within this BUILD.
	if f != nil {
		for _, d := range f.Directives {
			switch d.Key {
			case Directive_GitIgnore:
				enabled := common.ReadEnabled(d)
				c.Exts[enabledExt] = enabled
				if enabled {
					c.Exts[ASPECT_GITIGNORE] = createMatcherFunc(c)
				} else {
					c.Exts[ASPECT_GITIGNORE] = nil
				}
			}
		}
	}
}

func isEnabled(c *config.Config) bool {
	enabled, hasEnabled := c.Exts[enabledExt]
	return hasEnabled && enabled.(bool)
}

func addIgnore(c *config.Config, rel string, ignoreReader io.Reader) {
	var ignorePatterns []gitignore.Pattern

	// Load parent ignore patterns
	if c.Exts[ignorePatternsExt] != nil {
		ignorePatterns = c.Exts[ignorePatternsExt].([]gitignore.Pattern)
	}

	// Append new ignore patterns
	ignorePatterns = append(ignorePatterns, parseIgnore(rel, ignoreReader)...)

	// Persist appended ignore patterns
	c.Exts[ignorePatternsExt] = ignorePatterns

	// Persist a matcher function with the updated ignore patterns if enabled
	if isEnabled(c) {
		c.Exts[ASPECT_GITIGNORE] = createMatcherFunc(c)
	}
}

func parseIgnore(rel string, ignoreReader io.Reader) []gitignore.Pattern {
	var domain []string
	if rel != "" {
		domain = strings.Split(path.Clean(rel), "/")
	}

	matcherPatterns := make([]gitignore.Pattern, 0)

	reader := bufio.NewScanner(ignoreReader)
	for reader.Scan() {
		p := strings.TrimSpace(reader.Text())
		if p == "" || strings.HasPrefix(p, "#") {
			continue
		}

		matcherPatterns = append(matcherPatterns, gitignore.ParsePattern(p, domain))
	}

	return matcherPatterns
}

func createMatcherFunc(c *config.Config) isGitIgnoredFunc {
	patterns, patternsFound := c.Exts[ignorePatternsExt]
	if !patternsFound {
		return nil
	}

	matcher := gitignore.NewMatcher(patterns.([]gitignore.Pattern))
	return func(s string, isDir bool) bool {
		return matcher.Match(strings.Split(s, "/"), isDir)
	}
}
