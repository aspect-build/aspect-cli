package git

import (
	"bufio"
	"io"
	"os"
	"path"
	"strings"

	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/config"
	gitignore "github.com/go-git/go-git/plumbing/format/gitignore"
)

// Wrap the ignore files along with the relative path they were loaded from
// to enable quick-exit checks.
type ignoreEntry struct {
	i    gitignore.Matcher
	base string
}

type GitIgnore struct {
	ignores []ignoreEntry
}

func NewGitIgnore() *GitIgnore {
	return &GitIgnore{
		ignores: make([]ignoreEntry, 0),
	}
}

func (i *GitIgnore) CollectIgnoreFiles(c *config.Config, rel string) {
	// Collect gitignore style ignore files in this directory.
	ignoreFilePath := path.Join(c.RepoRoot, rel, ".gitignore")

	if ignoreReader, ignoreErr := os.Open(ignoreFilePath); ignoreErr == nil {
		BazelLog.Tracef("Add ignore file %s/.gitignore", rel)

		i.addIgnore(rel, ignoreReader)
	}
}

func (i *GitIgnore) addIgnore(rel string, ignoreReader io.Reader) {
	// Persist a relative path to the ignore file to enable quick-exit checks.
	base := path.Clean(rel)
	if base == "." {
		base = ""
	}

	domain := []string{}
	if base != "" {
		domain = strings.Split(base, "/")
	}

	matcherPatterns := make([]gitignore.Pattern, 0)

	reader := bufio.NewScanner(ignoreReader)
	for reader.Scan() {
		p := gitignore.ParsePattern(reader.Text(), domain)
		matcherPatterns = append(matcherPatterns, p)
	}

	ignore := gitignore.NewMatcher(matcherPatterns)

	// Add a trailing slash to the base path to ensure the ignore file only
	// processes paths within that directory.
	if base != "" && !strings.HasSuffix(base, "/") {
		base += "/"
	}

	i.ignores = append(i.ignores, ignoreEntry{
		i:    ignore,
		base: base,
	})
}

func (i *GitIgnore) Matches(p string) bool {
	for _, ignore := range i.ignores {
		// Ensure the path is within the base path of the ignore file
		// to avoid strings.Split unless necessary.
		if !strings.HasPrefix(p, ignore.base) {
			continue
		}
		if ignore.i.Match(strings.Split(p, "/"), false) {
			return true
		}
	}

	return false
}
