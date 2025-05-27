//go:build linux || darwin
// +build linux darwin

package python

import (
	python "github.com/bazel-contrib/rules_python/gazelle/python"
	"github.com/bazelbuild/bazel-gazelle/language"
)

func NewLanguage() language.Language {
	return python.NewLanguage()
}
