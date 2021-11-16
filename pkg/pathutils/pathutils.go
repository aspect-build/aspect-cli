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

// IsValidPackage returns true iff a file named BUILD or BUILD.bazel exists
// within the dir at the specified path
func IsValidPackage(path string) bool {
	return IsValidFile(filepath.Join(path, "BUILD")) ||
		IsValidFile(filepath.Join(path, "BUILD.bazel"))
}

func FindWorkspaceRoot(path string) string {
	if IsValidWorkspace(path) {
		return path
	}

	parentDirectory := filepath.Dir(path)
	if parentDirectory == path {
		return ""
	}

	return FindWorkspaceRoot(parentDirectory)
}

func FindNearestParentPackage(path string) string {
	if IsValidPackage(path) {
		return path
	}

	parentDirectory := filepath.Dir(path)
	if parentDirectory == path {
		return ""
	}

	return FindNearestParentPackage(parentDirectory)
}
