package gazelle

import (
	"sync"

	_ "unsafe"

	_ "github.com/bazelbuild/bazel-gazelle/walk"
)

// Required for using go:linkname below for using the private isExcluded.
// https://github.com/bazelbuild/bazel-gazelle/blob/v0.28.0/walk/config.go#L54-L73
type walkConfig struct {
	excludes []string
	// Below are fields that are not used by the isExcluded function but match the walkConfig
	// upstream walk.(*walkConfig).
	_ bool      // ignore bool
	_ []string  // follow []string
	_ sync.Once // loadOnce sync.Once
}

//go:linkname isExcluded github.com/bazelbuild/bazel-gazelle/walk.(*walkConfig).isExcluded
func isExcluded(wc *walkConfig, rel, base string) bool

func IsFileExcluded(rel, fileRelPath string, excludes []string) bool {
	// Gazelle exclude directive.
	wc := &walkConfig{excludes: excludes}

	return isExcluded(wc, rel, fileRelPath)
}
