/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package pathutils

import (
	"os"
	"path/filepath"
)

func IsValidFile(path string) bool {
	info, err := os.Stat(path)
	if err != nil {
		return false
	}

	return !info.IsDir()
}

// isValidWorkspace returns true iff the supplied path is the workspace root, defined by the presence of
// a file named WORKSPACE or WORKSPACE.bazel
// see https://github.com/bazelbuild/bazel/blob/8346ea4cfdd9fbd170d51a528fee26f912dad2d5/src/main/cpp/workspace_layout.cc#L37
func IsValidWorkspace(path string) bool {
	return IsValidFile(filepath.Join(path, "WORKSPACE")) ||
		IsValidFile(filepath.Join(path, "WORKSPACE.bazel"))
}

func FindWorkspaceRoot(root string) string {
	if IsValidWorkspace(root) {
		return root
	}

	parentDirectory := filepath.Dir(root)
	if parentDirectory == root {
		return ""
	}

	return FindWorkspaceRoot(parentDirectory)
}
