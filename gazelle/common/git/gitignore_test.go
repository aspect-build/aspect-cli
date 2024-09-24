package git

import (
	"fmt"
	"strings"
	"testing"

	"github.com/bazelbuild/bazel-gazelle/config"
)

func TestGitIgnore(t *testing.T) {
	shouldMatch := func(what string, c *config.Config, matches ...string) {
		matcher := createMatcherFunc(c)
		for _, m := range matches {
			if !(matcher != nil && matcher(m)) {
				t.Error(fmt.Sprintf("%s should match '%s'", what, m))
			}
		}
	}
	shouldNotMatch := func(what string, c *config.Config, matches ...string) {
		matcher := createMatcherFunc(c)
		for _, m := range matches {
			if matcher != nil && matcher(m) {
				t.Error(fmt.Sprintf("%s should NOT match '%s'", what, m))
			}
		}
	}

	t.Run("basic", func(t *testing.T) {
		c := config.New()
		addIgnoreFileContent(c, "", `
			# comment, that is indented

			a.js
			b/c.js
		`)

		shouldMatch("exact matches", c, "a.js", "b/c.js")
	})

	t.Run("partial paths", func(t *testing.T) {
		c := config.New()
		addIgnoreFileContent(c, "", `
			a.js
			b/c.js
			# b.js
		`)

		shouldNotMatch("partial matches", c, "a", "b", "b/c", "c.js")
		shouldNotMatch("files in comments", c, "b.js")
	})

	t.Run("nested ignore matches", func(t *testing.T) {
		c := config.New()
		addIgnoreFileContent(c, "", `
			a.js
		`)
		addIgnoreFileContent(c, "b", `
		    c.js
		`)

		shouldMatch("subdirectory patterns", c, "a.js", "b/c.js")
	})

	t.Run("overlapping ignore matches", func(t *testing.T) {
		c := config.New()
		addIgnoreFileContent(c, "", `
			a.js
			b/c/d/e.js
		`)
		addIgnoreFileContent(c, "b", `
		    asdf.js
		`)

		shouldMatch("overlapping paths", c, "a.js", "b/c/d/e.js", "b/asdf.js")
		shouldNotMatch("subdir on parent dir pattrn", c, "asdf.js")
	})

	t.Run("star dot", func(t *testing.T) {
		c := config.New()
		addIgnoreFileContent(c, "", `
			*.js
			a/*.js
		`)

		shouldMatch("star dot", c, "a.js", "A.js", "_.js", "b.js")
		shouldMatch("subdir star dot", c, "a/b.js", "a/abcd.js", "a/_.js", "a/.js", ".js", "x/y/z/a/b.js")
		shouldNotMatch("partial star dot", c, "a", "a/", "a.jsx", "a/b")
	})

	t.Run("stars", func(t *testing.T) {
		c := config.New()
		addIgnoreFileContent(c, "", `
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

		shouldMatch("exact", c, "r1.ts", "x/r1.ts", "subdir/direct-ig.ts")
		shouldNotMatch("different dirs", c, "othersub/direct-ig.ts", "x/subdir/direct-ig.ts", "subdir/x/direct-ig.ts")

		shouldMatch("star", c, "x/star-ig.ts")
		shouldNotMatch("start missing dir", c, "star-ig.ts", "subdir/x/star-ig.ts", "a/b/c/x/star-ig.ts")

		shouldMatch("double wildcard", c, "x.starstar-ig.ts", "subdir/x.starstar-ig.ts", "a/b/c/x.starstar-ig.ts", "a/.starstar-ig.ts")
		shouldNotMatch("double wildcard", c, ".startstar-ig.ts", "subdir/.startstar-ig.ts", "a/starstar-ig.ts")

		shouldMatch("all within", c, "all-within/x.ts", "all-within/subdir/x.ts", "all-within/a/b/c/x.ts")
		shouldNotMatch("all within", c, "x/all-within/x.tsx", "y/all-within/subdir/x.tsx")
	})

	t.Run("subdir matches", func(t *testing.T) {
		c := config.New()
		addIgnoreFileContent(c, "subdir", `
			node_modules
			dir_slash/
			dir_slash_star/*
			dir_slash_doublestar/**
		`)

		shouldMatch("no slash", c, "subdir/node_modules", "subdir/node_modules/x.ts", "subdir/node_modules/m/x.ts")

		shouldMatch("slash", c, "subdir/dir_slash/", "subdir/dir_slash/x.ts", "subdir/dir_slash/m/x.ts")
		shouldMatch("slash star", c, "subdir/dir_slash_star/x.ts", "subdir/dir_slash_star/m/x.ts")

		shouldNotMatch("slash star must have star content", c, "subdir/dir_slash_star")

		shouldMatch("slash double star", c, "subdir/dir_slash_doublestar/x.ts", "subdir/dir_slash_doublestar/m/x.ts")
		shouldMatch("slash double star does not require content", c, "subdir/dir_slash_doublestar")
	})

	t.Run("subdir matches all", func(t *testing.T) {
		c := config.New()
		addIgnoreFileContent(c, "subdir", "*")

		shouldMatch("all", c, "subdir/x", "subdir/x/y", "subdir/a.b", "subdir/a/b.c")
		shouldNotMatch("other dirs", c, "x", "x.y", "b/subdir", "b/subdir/x")
	})

	t.Run("subdir matches exact name", func(t *testing.T) {
		c := config.New()
		addIgnoreFileContent(c, "subdir", `
			r1.ts
			sub2/r2.ts
		`)

		shouldMatch("exact name abs", c, "subdir/r1.ts", "subdir/deeper/sub/dir/r1.ts", "subdir/sub2/r2.ts")
		shouldNotMatch("different dirs", c, "r1.ts", "othersub/r1.ts", "r2.ts", "othersub/r2.ts", "subdir/r2.ts", "subdir/other/r2.ts")
	})

	t.Run("stars subdir", func(t *testing.T) {
		c := config.New()
		addIgnoreFileContent(c, "subdir", `
			*/star.ts
			**/starstarslash.ts
		`)

		shouldMatch("star", c, "subdir/x/star.ts")
		shouldNotMatch("start different dirs", c, "star.ts", "a/b/c/x/star.ts")
		shouldNotMatch("start different subdirs", c, "subdir/x/y/star.ts")

		shouldMatch("double wildcard slash", c, "subdir/starstarslash.ts", "subdir/a/starstarslash.ts", "subdir/a/b/c/starstarslash.ts")
		shouldNotMatch("double wildcard slash different name pre", c, "subdir/x.starstarslash.ts", "subdir/.starstarslash.ts")
		shouldNotMatch("double wildcard slash different subdir", c, "a/x.starstarslash.ts", "a/b/c/starstarslash.ts")
	})

	t.Run("doublestar no slash", func(t *testing.T) {
		c := config.New()
		addIgnoreFileContent(c, "subdir", `
			**starstar.ts
		`)

		shouldMatch("double wildcard", c, "subdir/x.starstar.ts", "subdir/a/b/c/x.starstar.ts", "subdir/a/.starstar.ts")
		shouldNotMatch("double wildcard different subdir", c, "startstar.ts", ".startstar.ts", "a/starstar.ts")
	})
}

// Util method to invoke GitIgnore.AddIgnore() with the trimmed string
// value to allow tests to be written with multiline strings including indentation.
func addIgnoreFileContent(c *config.Config, rel, ignoreContents string) {
	ignoreLines := make([]string, 0)
	for _, line := range strings.Split(ignoreContents, "\n") {
		if trimmdLine := strings.TrimSpace(line); trimmdLine != "" {
			ignoreLines = append(ignoreLines, trimmdLine)
		}
	}

	addIgnore(c, rel, strings.NewReader(strings.Join(ignoreLines, "\n")))
}
