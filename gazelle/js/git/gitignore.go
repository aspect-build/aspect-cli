package gazelle

import (
	"os"
	"path"
	"strings"

	gitignore "github.com/sabhiram/go-gitignore"
)

type GitIgnore struct {
	ignores map[string][]*gitignore.GitIgnore
}

func NewGitIgnore() *GitIgnore {
	return &GitIgnore{
		ignores: make(map[string][]*gitignore.GitIgnore),
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
