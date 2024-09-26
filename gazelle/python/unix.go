//go:build linux || darwin
// +build linux darwin

package python

import (
	"github.com/bazelbuild/bazel-gazelle/language"
	python "github.com/bazelbuild/rules_python/gazelle/python"
)

func NewLanguage() language.Language {
	return python.NewLanguage()
}
