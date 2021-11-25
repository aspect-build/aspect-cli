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

// RunFn is the function signature for the cobra RunE function in the cobra.Command.
type RunFn func(cmd *cobra.Command, args []string) (exitErr error)

// InvokeCmdInsideWorkspace verifies if the current working directory is inside a Bazel workspace,
// then invokes the provided toRun function.
func InvokeCmdInsideWorkspace(toRun RunFn) RunFn {
	return invokeCmdInsideWorkspace(os.Getwd, defaultWorkspaceFinder, toRun)
}

func invokeCmdInsideWorkspace(
	osGetwd func() (string, error),
	finder Finder,
	toRun RunFn,
) RunFn {
	return func(cmd *cobra.Command, args []string) (exitErr error) {
		cwd, err := osGetwd()
		if err != nil {
			return fmt.Errorf("failed to run command %q inside workspace: %w", cmd.Use, err)
		}
		workspacePath, err := finder.Find(cwd)
		if err != nil {
			return fmt.Errorf("failed to run command %q inside workspace: %w", cmd.Use, err)
		}
		if workspacePath == "" {
			err = fmt.Errorf("the current working directory %q is not a bazel workspace "+
				"(below a directory having a WORKSPACE file)", cwd)
			return fmt.Errorf("failed to run command %q inside workspace: %w", cmd.Use, err)
		}
		return toRun(cmd, args)
	}
}

type Finder interface {
	Find(cwd string) (string, error)
}

// WorkspaceFinder is a struct that wraps the simple Find method that performs the
// finding of the WORKSPACE file in the user's Bazel project.
type WorkspaceFinder struct {
	osStat func(string) (fs.FileInfo, error)
}

var defaultWorkspaceFinder = &WorkspaceFinder{osStat: os.Stat}

// Find tries to find a file that marks the root of a Bazel workspace (WORKSPACE or WORKSPACE.bazel). If the returned
// path is empty and no error was produced, the user's current working directory
// is not a Bazel workspace.
func (f *WorkspaceFinder) Find(cwd string) (string, error) {
	for {
		if cwd == "." || cwd == filepath.Dir(cwd) {
			return "", nil
		}
		for _, workspaceFilename := range WorkspaceFilenames {
			workspacePath := path.Join(cwd, workspaceFilename)
			if _, err := f.osStat(workspacePath); err == nil {
				return workspacePath, nil
			} else if !os.IsNotExist(err) {
				return "", fmt.Errorf("failed to find bazel workspace: %w", err)
			}
		}
		cwd = filepath.Dir(cwd)
	}
}
