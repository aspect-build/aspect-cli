package git

import (
	"fmt"
	"strings"
	"testing"

	gitignore "github.com/go-git/go-git/v5/plumbing/format/gitignore"
)

func TestGitIgnore(t *testing.T) {
	shouldMatch := func(what string, matcher isGitIgnored, matches ...string) {
		for _, m := range matches {
			// Use trailing slash in test data to indicate directory
			isDir := strings.HasSuffix(m, "/")
			m = strings.TrimSuffix(m, "/")

			if !(matcher != nil && matcher(m, isDir)) {
				t.Error(fmt.Sprintf("%s should match '%s'", what, m))
			}
		}
	}
	shouldNotMatch := func(what string, matcher isGitIgnored, matches ...string) {
		for _, m := range matches {
			// Use trailing slash in test data to indicate directory
			isDir := strings.HasSuffix(m, "/")
			m = strings.TrimSuffix(m, "/")

			if matcher != nil && matcher(m, isDir) {
				t.Error(fmt.Sprintf("%s should NOT match '%s'", what, m))
			}
		}
	}

	t.Run("basic", func(t *testing.T) {
		m, _ := addIgnoreFileContent(nil, "", `
			# comment, that is indented

			a.js
			b/c.js
		`)

		shouldMatch("exact matches", m, "a.js", "b/c.js")
	})

	t.Run("partial paths", func(t *testing.T) {
		m, _ := addIgnoreFileContent(nil, "", `
			a.js
			b/c.js
			# b.js
		`)

		shouldNotMatch("partial matches", m, "a", "b", "b/c", "c.js")
		shouldNotMatch("files in comments", m, "b.js")
	})

	t.Run("nested ignore matches", func(t *testing.T) {
		m, p := addIgnoreFileContent(nil, "", `
			a.js
		`)
		m, _ = addIgnoreFileContent(p, "b", `
		    c.js
		`)

		shouldMatch("subdirectory patterns", m, "a.js", "b/c.js")
	})

	t.Run("overlapping ignore matches", func(t *testing.T) {
		m, p := addIgnoreFileContent(nil, "", `
			a.js
			b/c/d/e.js
		`)
		m, _ = addIgnoreFileContent(p, "b", `
		    asdf.js
		`)

		shouldMatch("overlapping paths", m, "a.js", "b/c/d/e.js", "b/asdf.js")
		shouldNotMatch("subdir on parent dir pattrn", m, "asdf.js")
	})

	t.Run("star dot", func(t *testing.T) {
		m, _ := addIgnoreFileContent(nil, "", `
			*.js
			a/*.js
		`)

		shouldMatch("star dot", m, "a.js", "A.js", "_.js", "b.js")
		shouldMatch("subdir star dot", m, "a/b.js", "a/abcd.js", "a/_.js", "a/.js", ".js", "x/y/z/a/b.js")
		shouldNotMatch("partial star dot", m, "a", "a/", "a.jsx", "a/b")
	})

	t.Run("stars", func(t *testing.T) {
		m, _ := addIgnoreFileContent(nil, "", `
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

		shouldMatch("exact", m, "r1.ts", "x/r1.ts", "subdir/direct-ig.ts")
		shouldNotMatch("different dirs", m, "othersub/direct-ig.ts", "x/subdir/direct-ig.ts", "subdir/x/direct-ig.ts")

		shouldMatch("star", m, "x/star-ig.ts")
		shouldNotMatch("start missing dir", m, "star-ig.ts", "subdir/x/star-ig.ts", "a/b/c/x/star-ig.ts")

		shouldMatch("double wildcard", m, "x.starstar-ig.ts", "subdir/x.starstar-ig.ts", "a/b/c/x.starstar-ig.ts", "a/.starstar-ig.ts")
		shouldNotMatch("double wildcard", m, ".startstar-ig.ts", "subdir/.startstar-ig.ts", "a/starstar-ig.ts")

		shouldMatch("all within", m, "all-within/x.ts", "all-within/subdir/x.ts", "all-within/a/b/c/x.ts")
		shouldNotMatch("all within", m, "x/all-within/x.tsx", "y/all-within/subdir/x.tsx")
	})

	t.Run("subdir matches", func(t *testing.T) {
		m, _ := addIgnoreFileContent(nil, "subdir", `
			node_modules
			dir_slash/
			dir_slash_star/*
			dir_slash_doublestar/**
		`)

		shouldMatch("no slash", m, "subdir/node_modules", "subdir/node_modules/x.ts", "subdir/node_modules/m/x.ts")

		shouldMatch("slash", m, "subdir/dir_slash/", "subdir/dir_slash/x.ts", "subdir/dir_slash/m/x.ts")
		shouldMatch("slash star", m, "subdir/dir_slash_star/x.ts", "subdir/dir_slash_star/m/x.ts")

		shouldNotMatch("slash star must have star content", m, "subdir/dir_slash_star")

		shouldMatch("slash double star", m, "subdir/dir_slash_doublestar/x.ts", "subdir/dir_slash_doublestar/m/x.ts")
		shouldMatch("slash double star does not require content", m, "subdir/dir_slash_doublestar")
	})

	t.Run("subdir matches all", func(t *testing.T) {
		m, _ := addIgnoreFileContent(nil, "subdir", "*")

		shouldMatch("all", m, "subdir/x", "subdir/x/y", "subdir/a.b", "subdir/a/b.c")
		shouldNotMatch("other dirs", m, "x", "x.y", "b/subdir", "b/subdir/x")
	})

	t.Run("subdir matches exact name", func(t *testing.T) {
		m, _ := addIgnoreFileContent(nil, "subdir", `
			r1.ts
			sub2/r2.ts
		`)

		shouldMatch("exact name abs", m, "subdir/r1.ts", "subdir/deeper/sub/dir/r1.ts", "subdir/sub2/r2.ts")
		shouldNotMatch("different dirs", m, "r1.ts", "othersub/r1.ts", "r2.ts", "othersub/r2.ts", "subdir/r2.ts", "subdir/other/r2.ts")
	})

	t.Run("stars subdir", func(t *testing.T) {
		m, _ := addIgnoreFileContent(nil, "subdir", `
			*/star.ts
			**/starstarslash.ts
		`)

		shouldMatch("star", m, "subdir/x/star.ts")
		shouldNotMatch("start different dirs", m, "star.ts", "a/b/c/x/star.ts")
		shouldNotMatch("start different subdirs", m, "subdir/x/y/star.ts")

		shouldMatch("double wildcard slash", m, "subdir/starstarslash.ts", "subdir/a/starstarslash.ts", "subdir/a/b/c/starstarslash.ts")
		shouldNotMatch("double wildcard slash different name pre", m, "subdir/x.starstarslash.ts", "subdir/.starstarslash.ts")
		shouldNotMatch("double wildcard slash different subdir", m, "a/x.starstarslash.ts", "a/b/c/starstarslash.ts")
	})

	t.Run("doublestar no slash", func(t *testing.T) {
		m, _ := addIgnoreFileContent(nil, "subdir", `
			**starstar.ts
		`)

		shouldMatch("double wildcard", m, "subdir/x.starstar.ts", "subdir/a/b/c/x.starstar.ts", "subdir/a/.starstar.ts")
		shouldNotMatch("double wildcard different subdir", m, "startstar.ts", ".startstar.ts", "a/starstar.ts")
	})

	t.Run("dir specific matches", func(t *testing.T) {
		m, _ := addIgnoreFileContent(nil, "", `
		    **/node_modules/
		    **/foo.js/
		`)

		shouldMatch("dir pattern", m, "node_modules/", "a/b/node_modules/")
		shouldMatch("dir pattern that looks like a file", m, "foo.js/", "a/b/foo.js/")
		shouldNotMatch("ending file that looks like dir", m, "node_modules", "x/node_modules", "foo.js", "x/foo.js")
	})
}

// Util method to invoke GitIgnore.AddIgnore() with the trimmed string
// value to allow tests to be written with multiline strings including indentation.
func addIgnoreFileContent(parentPatterns []gitignore.Pattern, rel, ignoreContents string) (isGitIgnored, []gitignore.Pattern) {
	ignoreLines := make([]string, 0)
	for _, line := range strings.Split(ignoreContents, "\n") {
		if trimmdLine := strings.TrimSpace(line); trimmdLine != "" {
			ignoreLines = append(ignoreLines, trimmdLine)
		}
	}

	patterns := parseIgnore(rel, strings.NewReader(strings.Join(ignoreLines, "\n")))
	patterns = append(parentPatterns, patterns...)

	return createMatcherFunc(patterns), patterns
}
