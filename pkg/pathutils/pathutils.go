/*
Copyright © 2021 Aspect Build Systems Inc
Not licensed for re-use.
*/

package pathutils

import (
	"fmt"
	"os"
	"path/filepath"
)

func IsFile(path string) bool {
	info, err := os.Stat(path)
	if err != nil {
		return false
	}

	return !info.IsDir()
}

// IsValidWorkspace isValidWorkspace returns true iff the supplied path is the workspace root,
// defined by the presence of a file named WORKSPACE or WORKSPACE.bazel
// see https://github.com/bazelbuild/bazel/blob/8346ea4cfdd9fbd170d51a528fee26f912dad2d5/src/main/cpp/workspace_layout.cc#L37
func IsValidWorkspace(path string) bool {
	return IsFile(filepath.Join(path, "WORKSPACE")) ||
		IsFile(filepath.Join(path, "WORKSPACE.bazel"))
}

// IsValidPackage returns true iff a file named BUILD or BUILD.bazel exists
// within the dir at the specified path
func IsValidPackage(path string) bool {
	return IsFile(filepath.Join(path, "BUILD")) ||
		IsFile(filepath.Join(path, "BUILD.bazel"))
}

func FindWorkspaceRoot(path string) string {
	if IsValidWorkspace(path) {
		return path
	}

	curPath := path
	parPath := filepath.Dir(curPath)
	// The stopping condition occurs when we've reached the root directory on disk,
	// ie. when the current folder's parent is itself.
	for parPath != curPath {
		curPath = parPath
		if IsValidWorkspace(curPath) {
			return curPath
		}
		parPath = filepath.Dir(curPath)
	}

	return ""
}

func InvokeCmdInsideWorkspace(cmdName string, fn func() error) error {
	workingDirectory, err := os.Getwd()
	if err != nil {
		return fmt.Errorf("could not resolve working directory: %w", err)
	}
	workspaceRoot := FindWorkspaceRoot(workingDirectory)
	if workspaceRoot == "" {
		return fmt.Errorf("the '%s' command is only supported from within a workspace " +
			"(below a directory having a WORKSPACE file)", cmdName)
	}
	err = fn()
	if err != nil {
		return err
	}
	return nil
}
