package git

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"path"
	"strings"

	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/walk"
	gitignore "github.com/go-git/go-git/v5/plumbing/format/gitignore"
)

// TODO: remove and align with gazelle after https://github.com/aspect-build/aspect-cli/issues/755

func SetupGitIgnore() {
	walk.SetGitIgnoreProcessor(processGitignoreFile)
}

type isGitIgnored func(p string, isDir bool) bool

func processGitignoreFile(rootDir, gitignorePath string, d interface{}) (func(p string, isDir bool) bool, interface{}) {
	var ignorePatterns []gitignore.Pattern
	if d != nil {
		ignorePatterns = d.([]gitignore.Pattern)
	}

	ignoreReader, ignoreErr := os.Open(path.Join(rootDir, gitignorePath))
	if ignoreErr == nil {
		BazelLog.Tracef("Add gitignore file %s", gitignorePath)
		defer ignoreReader.Close()

		ignorePatterns = append(ignorePatterns, parseIgnore(path.Dir(gitignorePath), ignoreReader)...)
	} else {
		msg := fmt.Sprintf("Failed to open %s: %v", gitignorePath, ignoreErr)
		BazelLog.Error(msg)
		fmt.Printf("%s\n", msg)
	}

	if len(ignorePatterns) == 0 {
		return nil, nil
	}

	// Trim the capacity of the slice to the length to ensure any additional
	// append()ing in the future will reallocate and copy the origina slice.
	ignorePatterns = ignorePatterns[:len(ignorePatterns):len(ignorePatterns)]

	return createMatcherFunc(ignorePatterns), ignorePatterns
}

func createMatcherFunc(ignorePatterns []gitignore.Pattern) isGitIgnored {
	matcher := gitignore.NewMatcher(ignorePatterns)
	return func(s string, isDir bool) bool {
		return matcher.Match(strings.Split(s, "/"), isDir)
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
