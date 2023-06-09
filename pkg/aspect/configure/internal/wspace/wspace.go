// Copy of gazelle internal https://github.com/bazelbuild/bazel-gazelle/blob/7feffe17f56e2b76eae8a4f2933215d6c5924176/internal/wspace/finder.go

// Package wspace provides functions to locate and modify a bazel WORKSPACE file.
package wspace

import (
	"os"
	"path/filepath"
)

func FindWORKSPACEFile(root string) string {
	pathWithExt := filepath.Join(root, "WORKSPACE.bazel")
	if _, err := os.Stat(pathWithExt); err == nil {
		return pathWithExt
	}
	return filepath.Join(root, "WORKSPACE")
}
