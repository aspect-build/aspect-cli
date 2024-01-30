package git

import (
	"io"
	"os"
	"path"
	"strings"

	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/config"
	gitignore "github.com/denormal/go-gitignore"
)

// Ignore files following .gitignore syntax for files gazelle will ignore.
var bazelIgnoreFiles = []string{".bazelignore", ".gitignore"}

// Wrap the ignore files along with the relative path they were loaded from
// to enable quick-exit checks.
type ignoreEntry struct {
	i    gitignore.GitIgnore
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
	for _, ignoreFileName := range bazelIgnoreFiles {
		ignoreRelPath := path.Join(rel, ignoreFileName)
		ignoreFilePath := path.Join(c.RepoRoot, ignoreRelPath)

		if ignoreReader, ignoreErr := os.Open(ignoreFilePath); ignoreErr == nil {
			BazelLog.Tracef("Add ignore file %s", ignoreRelPath)

			i.addIgnore(rel, ignoreReader)
		}
	}
}

func (i *GitIgnore) addIgnore(rel string, ignoreReader io.Reader) {
	// Persist a relative path to the ignore file to enable quick-exit checks.
	base := path.Clean(rel)
	if base == "." {
		base = ""
	}

	ignore := gitignore.New(ignoreReader, base, func(err gitignore.Error) bool {
		BazelLog.Warnf("Failed to parse ignore file: %v at %v", err, err.Position())
		return true
	})

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
		// Quick check to see if the ignore file could possibly match the path.
		if !strings.HasPrefix(p, ignore.base) {
			continue
		}

		if m := ignore.i.Relative(p, false); m != nil && m.Ignore() {
			return true
		}
	}

	return false
}
