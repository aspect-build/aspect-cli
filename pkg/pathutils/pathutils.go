/*
Copyright © 2021 Aspect Build Systems Inc
Not licensed for re-use.
*/

package pathutils

import (
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
)

func IsFile(path string) bool {
	info, err := os.Stat(path)
	if err != nil {
		return false
	}

	return !info.IsDir()
}

// IsWorkspace isValidWorkspace returns true iff the supplied path is the workspace root,
// defined by the presence of a file named WORKSPACE or WORKSPACE.bazel
// see https://github.com/bazelbuild/bazel/blob/8346ea4cfdd9fbd170d51a528fee26f912dad2d5/src/main/cpp/workspace_layout.cc#L37
func IsWorkspace(path string) bool {
	return IsFile(filepath.Join(path, "WORKSPACE")) ||
		IsFile(filepath.Join(path, "WORKSPACE.bazel"))
}

// IsPackage returns true iff a file named BUILD or BUILD.bazel exists
// within the dir at the specified path
func IsPackage(path string) bool {
	return IsFile(filepath.Join(path, "BUILD")) ||
		IsFile(filepath.Join(path, "BUILD.bazel"))
}

// GetBuildFilePath returns the path to the build file for a specified path to a package root
func GetBuildFilePath(pkgPath string) (string, error) {
	if IsFile(filepath.Join(pkgPath, "BUILD")) {
		return filepath.Join(pkgPath, "BUILD"), nil
	} else if IsFile(filepath.Join(pkgPath, "BUILD.bazel")) {
		return filepath.Join(pkgPath, "BUILD.bazel"), nil
	}
	return "", fmt.Errorf("supplied path is not a path to a package root")
}

func getFunctionName(i interface{}) string {
	return runtime.FuncForPC(reflect.ValueOf(i).Pointer()).Name()
}

func FindParentPathSatisfyingCondition(path string, condition func(string) bool) (string, error) {
	if condition(path) {
		return path, nil
	}

	curPath := path
	parPath := filepath.Dir(curPath)
	// The stopping condition occurs when we've reached the root directory on disk,
	// ie. when the current folder's parent is itself.
	for parPath != curPath {
		curPath = parPath
		if condition(curPath) {
			return curPath, nil
		}
		parPath = filepath.Dir(curPath)
	}

	return "", fmt.Errorf("no parent path found satisfying condition %v", getFunctionName(condition))
}

func FindWorkspaceRoot(path string) (string, error) {
	return FindParentPathSatisfyingCondition(path, IsWorkspace)
}

func FindNearestParentPackage(path string) (string, error) {
	return FindParentPathSatisfyingCondition(path, IsPackage)
}

func InvokeCmdInsideWorkspace(cmdName string, fn func() error) error {
	workingDirectory, err := os.Getwd()
	if err != nil {
		return fmt.Errorf("could not resolve working directory: %w", err)
	}
	_, err = FindWorkspaceRoot(workingDirectory)
	if err != nil {
		return fmt.Errorf("the '%s' command is only supported from within a workspace " +
			"(below a directory having a WORKSPACE file)", cmdName)
	}
	err = fn()
	if err != nil {
		return err
	}
	return nil
}

// GetAllInSpecifiedPackagePattern Returns a pattern for all targets within the specified package
func GetAllInSpecifiedPackagePattern(path string) (string, error) {
	workingDirectory, err := os.Getwd()
	workspaceRoot, err := FindWorkspaceRoot(workingDirectory)
	pathToPkg, err := FindNearestParentPackage(filepath.Join(workspaceRoot,path))
	if err != nil {
		return "", fmt.Errorf("GetAllInSpecifiedPackagePattern failed: %w", err)
	}
	if pathToPkg == workspaceRoot {
		// Current directory is the WORKSPACE root
		return "//:all", nil
	}
	pathToPkg, err = filepath.Rel(workspaceRoot, pathToPkg)
	if err != nil {
		return "", fmt.Errorf("prompt failed: %w", err)
	}
	return "//" + pathToPkg + ":all", nil
}

// GetAllInCurrentPackagePattern Returns a pattern for all targets within the current folder
func GetAllInCurrentPackagePattern() (string, error) {
	workingDirectory, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("prompt failed: %w", err)
	}
	return GetAllInSpecifiedPackagePattern(workingDirectory)
}
