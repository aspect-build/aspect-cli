/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package pathutils

import (
	"fmt"
	"io/fs"
	"os"
	"path"
	"path/filepath"

	"github.com/spf13/cobra"
)

// https://github.com/bazelbuild/bazel/blob/8346ea4c/src/main/cpp/workspace_layout.cc#L37
var WorkspaceFilenames = []string{"WORKSPACE", "WORKSPACE.bazel"}

// CobraRunEFn is the function signature for the cobra RunE function in the cobra.Command.
type CobraRunEFn func(cmd *cobra.Command, args []string) (exitErr error)

// RunWorkspaceFn is the function signature based on CobraRunEFn that includes the workspace root.
type RunWorkspaceFn func(workspaceRoot string, cmd *cobra.Command, args []string) (exitErr error)

// InvokeCmdInsideWorkspace verifies if the current working directory is inside a Bazel workspace,
// then invokes the provided toRun function, injecting the found workspace root path.
func InvokeCmdInsideWorkspace(toRun RunWorkspaceFn) CobraRunEFn {
	wd, err := os.Getwd()
	if err != nil {
		panic(err)
	}
	return invokeCmdInsideWorkspace(defaultWorkspaceFinder, wd, toRun)
}

func invokeCmdInsideWorkspace(
	finder Finder,
	wd string,
	toRun RunWorkspaceFn,
) CobraRunEFn {
	return func(cmd *cobra.Command, args []string) (exitErr error) {
		workspacePath, err := finder.Find(wd)
		if err != nil {
			return fmt.Errorf("failed to run command %q: %w", cmd.Use, err)
		}
		if workspacePath == "" {
			err = fmt.Errorf("the current working directory %q is not a Bazel workspace", wd)
			return fmt.Errorf("failed to run command %q: %w", cmd.Use, err)
		}
		workspaceRoot := path.Dir(workspacePath)
		return toRun(workspaceRoot, cmd, args)
	}
}

// Finder wraps the Find method that performs the finding of the WORKSPACE file
// in the user's Bazel project.
type Finder interface {
	Find(wd string) (string, error)
}

type workspaceFinder struct {
	osStat func(string) (fs.FileInfo, error)
}

var defaultWorkspaceFinder = &workspaceFinder{osStat: os.Stat}

// Find tries to find a file that marks the root of a Bazel workspace
// (WORKSPACE or WORKSPACE.bazel). If the returned path is empty and no error
// was produced, the user's current working directory is not a Bazel workspace.
func (f *workspaceFinder) Find(wd string) (string, error) {
	for {
		if wd == "." || wd == filepath.Dir(wd) {
			return "", nil
		}
		for _, workspaceFilename := range WorkspaceFilenames {
			workspacePath := path.Join(wd, workspaceFilename)
			fileInfo, err := f.osStat(workspacePath)
			if err != nil {
				if os.IsNotExist(err) {
					continue
				}
				return "", fmt.Errorf("failed to find bazel workspace: %w", err)
			}
			if fileInfo.IsDir() {
				continue
			}
			return workspacePath, nil
		}
		wd = filepath.Dir(wd)
	}
}
