package git

import (
	"fmt"
	"strings"
	"testing"
)

func TestGitIgnore(t *testing.T) {
	shouldMatch := func(what string, i *GitIgnore, matches ...string) {
		for _, m := range matches {
			if !i.Matches(m) {
				t.Error(fmt.Sprintf("%s should match '%s'", what, m))
			}
		}
	}
	shouldNotMatch := func(what string, i *GitIgnore, matches ...string) {
		for _, m := range matches {
			if i.Matches(m) {
				t.Error(fmt.Sprintf("%s should NOT match '%s'", what, m))
			}
		}
	}

	t.Run("basic", func(t *testing.T) {
		i := NewGitIgnore()
		addIgnoreFileContent(i, ".", `
			# comment, that is indented

			a.js
			b/c.js
		`)

		shouldMatch("exact matches", i, "a.js", "b/c.js")
	})

	t.Run("partial paths", func(t *testing.T) {
		i := NewGitIgnore()
		addIgnoreFileContent(i, ".", `
			a.js
			b/c.js
			# b.js
		`)

		shouldNotMatch("partial matches", i, "a", "b", "b/c", "c.js")
		shouldNotMatch("files in comments", i, "b.js")
	})

	t.Run("nested ignore matches", func(t *testing.T) {
		i := NewGitIgnore()
		addIgnoreFileContent(i, ".", `
			a.js
		`)
		addIgnoreFileContent(i, "b", `
		    c.js
		`)

		shouldMatch("subdirectory patterns", i, "a.js", "b/c.js")
	})

	t.Run("overlapping ignore matches", func(t *testing.T) {
		i := NewGitIgnore()
		addIgnoreFileContent(i, ".", `
			a.js
			b/c/d/e.js
		`)
		addIgnoreFileContent(i, "b", `
		    asdf.js
		`)

		shouldMatch("overlapping paths", i, "a.js", "b/c/d/e.js", "b/asdf.js")
		shouldNotMatch("subdir on parent dir pattrn", i, "asdf.js")
	})

	t.Run("star dot", func(t *testing.T) {
		i := NewGitIgnore()
		addIgnoreFileContent(i, ".", `
			*.js
			a/*.js
		`)

		shouldMatch("star dot", i, "a.js", "A.js", "_.js", "b.js")
		shouldMatch("subdir star dot", i, "a/b.js", "a/abcd.js", "a/_.js", "a/.js", ".js", "x/y/z/a/b.js")
		shouldNotMatch("partial star dot", i, "a", "a/", "a.jsx", "a/b")
	})

	t.Run("stars", func(t *testing.T) {
		i := NewGitIgnore()
		addIgnoreFileContent(i, ".", `
			# A file by name only in root
			r1.ts

			# A file within a subdir ignored by the root
			subdir/direct-ig.ts
			\n
			# Files within any sub
			*/star-ig.ts
\t
			# A global glob configured from the root
			**/*.starstar-ig.ts

			all-within/**
		`)

		shouldMatch("exact", i, "r1.ts", "x/r1.ts", "subdir/direct-ig.ts")
		shouldNotMatch("different dirs", i, "othersub/direct-ig.ts", "x/subdir/direct-ig.ts", "subdir/x/direct-ig.ts")

		shouldMatch("star", i, "x/star-ig.ts")
		shouldNotMatch("start missing dir", i, "star-ig.ts", "subdir/x/star-ig.ts", "a/b/c/x/star-ig.ts")

		shouldMatch("double wildcard", i, "x.starstar-ig.ts", "subdir/x.starstar-ig.ts", "a/b/c/x.starstar-ig.ts", "a/.starstar-ig.ts")
		shouldNotMatch("double wildcard", i, ".startstar-ig.ts", "subdir/.startstar-ig.ts", "a/starstar-ig.ts")

		shouldMatch("all within", i, "all-within/x.ts", "all-within/subdir/x.ts", "all-within/a/b/c/x.ts")
		shouldNotMatch("all within", i, "x/all-within/x.tsx", "y/all-within/subdir/x.tsx")
	})
}

// Util method to invoke GitIgnore.AddIgnore() with the trimmed string
// value to allow tests to be written with multiline strings including indentation.
func addIgnoreFileContent(i *GitIgnore, rel, ignoreContents string) {
	ignoreLines := make([]string, 0)
	for _, line := range strings.Split(ignoreContents, "\n") {
		if trimmdLine := strings.TrimSpace(line); trimmdLine != "" {
			ignoreLines = append(ignoreLines, trimmdLine)
		}
	}

	i.addIgnore(rel, strings.NewReader(strings.Join(ignoreLines, "\n")))
}
