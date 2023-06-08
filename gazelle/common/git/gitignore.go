package git

import (
	"os"
	"path"
	"strings"

	. "aspect.build/cli/gazelle/common/log"
	"github.com/bazelbuild/bazel-gazelle/config"
	gitignore "github.com/sabhiram/go-gitignore"
)

// Ignore files following .gitignore syntax for files gazelle will ignore.
var bazelIgnoreFiles = []string{".bazelignore", ".gitignore"}

type GitIgnore struct {
	ignores map[string][]*gitignore.GitIgnore
}

func NewGitIgnore() *GitIgnore {
	return &GitIgnore{
		ignores: make(map[string][]*gitignore.GitIgnore),
	}
}

func (i *GitIgnore) CollectIgnoreFiles(c *config.Config, rel string) {
	// Collect gitignore style ignore files in this directory.
	for _, ignoreFileName := range bazelIgnoreFiles {
		ignoreRelPath := path.Join(rel, ignoreFileName)
		ignoreFilePath := path.Join(c.RepoRoot, ignoreRelPath)

		if _, ignoreErr := os.Stat(ignoreFilePath); ignoreErr == nil {
			BazelLog.Tracef("Add ignore file %s", ignoreRelPath)

			ignoreErr := i.AddIgnoreFile(rel, ignoreFilePath)
			if ignoreErr != nil {
				BazelLog.Fatalf("Failed to add ignore file %s: %v", ignoreRelPath, ignoreErr)
			}
		}
	}
}

// Add the given ignore file rules relative to the given directory.
func (i *GitIgnore) AddIgnoreFile(ignoreDir, ignoreFile string) error {
	contents, err := os.ReadFile(ignoreFile)
	if err != nil {
		return err
	}

	i.addIgnoreFileContent(ignoreDir, strings.Split(string(contents), "\n"))
	return nil
}

func (i *GitIgnore) addIgnoreFileContent(ignoreDir string, ignoreContents []string) {
	ignoreDir = path.Clean(ignoreDir)

	if i.ignores[ignoreDir] == nil {
		i.ignores[ignoreDir] = make([]*gitignore.GitIgnore, 0)
	}

	ignore := gitignore.CompileIgnoreLines(ignoreContents...)

	i.ignores[ignoreDir] = append(i.ignores[ignoreDir], ignore)
}

func (i *GitIgnore) Matches(p string) bool {
	p = path.Clean(p)

	for d, f := path.Dir(p), path.Base(p); ; d, f = path.Dir(d), path.Join(path.Base(d), f) {
		if ignores := i.ignores[d]; ignores != nil {
			for _, ignore := range ignores {
				if ignore.MatchesPath(f) {
					return true
				}
			}
		}

		if d == "." {
			return false
		}
	}
}
