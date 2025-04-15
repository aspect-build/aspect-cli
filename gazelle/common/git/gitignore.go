package git

import (
	"bufio"
	"fmt"
	"io"
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
const ignorePatternsExt = "gitignore:patterns"

func SetupGitConfig(rootConfig *config.Config) {
	rootConfig.Exts["__aspect:processGitignoreFile"] = processGitignoreFile
}

func processGitignoreFile(c *config.Config, p string) {
	ignoreFilePath := path.Join(c.RepoRoot, p)
	ignoreReader, ignoreErr := os.Open(ignoreFilePath)
	if ignoreErr == nil {
		BazelLog.Tracef("Add gitignore file %s", p)
		defer ignoreReader.Close()
		addIgnore(c, path.Dir(p), ignoreReader)
	} else {
		msg := fmt.Sprintf("Failed to open %s: %v", p, ignoreErr)
		BazelLog.Error(msg)
		fmt.Printf("%s\n", msg)
	}
}

func ReadGitConfig(c *config.Config, rel string, f *rule.File) {
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
	if rel != "" && rel != "." {
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
