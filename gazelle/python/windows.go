//go:build windows
// +build windows

package python

import (
	"log"

	"github.com/bazelbuild/bazel-gazelle/language"
)

func NewLanguage() language.Language {
	log.Fatalln("Python extension is not supported on Windows.\nSee: https://github.com/aspect-build/aspect-cli/issues/747")
	return nil
}
