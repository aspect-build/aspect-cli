// Copy of gazelle internal https://github.com/bazelbuild/bazel-gazelle/blob/b62589672b5c32264ddf40585247d684c29bdd15/internal/wspace/finder.go

// Package wspace provides functions to locate and modify a bazel WORKSPACE file.
package wspace

import (
	"os"
	"path/filepath"
)

var workspaceFiles = []string{"WORKSPACE.bazel", "WORKSPACE"}

// IsWORKSPACE checks whether path is named WORKSPACE or WORKSPACE.bazel
func IsWORKSPACE(path string) bool {
	base := filepath.Base(path)
	for _, workspaceFile := range workspaceFiles {
		if base == workspaceFile {
			return true
		}
	}
	return false
}

// FindWORKSPACEFile returns a path to a file in the provided root directory,
// either to an existing WORKSPACE or WORKSPACE.bazel file, or to root/WORKSPACE
// if neither exists. Note that this function does NOT recursively check parent directories.
func FindWORKSPACEFile(root string) string {
	for _, workspaceFile := range workspaceFiles {
		path := filepath.Join(root, workspaceFile)
		if fileInfo, err := os.Stat(path); err == nil && !fileInfo.IsDir() {
			return path
		}
	}
	return filepath.Join(root, "WORKSPACE")
}
