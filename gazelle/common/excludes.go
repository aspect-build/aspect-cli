package gazelle

import (
	_ "unsafe"

	_ "github.com/bazelbuild/bazel-gazelle/walk"
)

// Required for using go:linkname below for using the private isExcluded.
// walkConfig: https://github.com/bazelbuild/bazel-gazelle/blob/v0.38.0/walk/config.go#L41-L46
type walkConfig struct {
	excludes []string
	// Below are fields that are not used by the isExcluded function but match the walkConfig
	// upstream walk.(*walkConfig).
	_ bool     // ignore bool
	_ []string // follow []string
}

// isExcluded: https://github.com/bazelbuild/bazel-gazelle/blob/v0.38.0/walk/config.go#L54-L59
//
//go:linkname isExcluded github.com/bazelbuild/bazel-gazelle/walk.(*walkConfig).isExcluded
func isExcluded(wc *walkConfig, rel, base string) bool

func IsFileExcluded(rel, fileRelPath string, excludes []string) bool {
	if len(excludes) == 0 {
		return false
	}

	// Gazelle exclude directive.
	wc := &walkConfig{excludes: excludes}

	return isExcluded(wc, rel, fileRelPath)
}
