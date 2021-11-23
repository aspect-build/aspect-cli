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
var workspaceFilenames = []string{"WORKSPACE", "WORKSPACE.bazel"}

type RunFn func(cmd *cobra.Command, args []string) (exitErr error)

func InvokeCmdInsideWorkspace(toRun RunFn) (wrapped RunFn) {
	return func(cmd *cobra.Command, args []string) (exitErr error) {
		finder := NewDefaultWorkspaceFinder()
		cwd, err := os.Getwd()
		if err != nil {
			return fmt.Errorf("failed to run command %q inside workspace: %w", cmd.Use, err)
		}
		workspacePath, err := finder.Find(cwd)
		if err != nil {
			return fmt.Errorf("failed to run command %q inside workspace: %w", cmd.Use, err)
		}
		if workspacePath == "" {
			err = fmt.Errorf("the current working directory %q is not a bazel workspace", cwd)
			return fmt.Errorf("failed to run command %q inside workspace: %w", cmd.Use, err)
		}
		return toRun(cmd, args)
	}
}

// WorkspaceFinder is the interface that wraps the simple Find method that performs the
// finding of the WORKSPACE file in the user's Bazel project.
type WorkspaceFinder struct {
	osGetwd func() (string, error)
	osStat  func(string) (fs.FileInfo, error)
}

// NewDefaultWorkspaceFinder instantiates a default internal implementation of the WorkspaceFinder
// interface.
func NewDefaultWorkspaceFinder() *WorkspaceFinder {
	return &WorkspaceFinder{
		osGetwd: os.Getwd,
		osStat:  os.Stat,
	}
}

// Find finds the WORKSPACE file under a Bazel workspace. If the returned
// path is empty and no error was produced, the user's current working directory
// is not a Bazel workspace.
func (f *WorkspaceFinder) Find(cwd string) (string, error) {
	for {
		if cwd == "." {
			return "", nil
		}
		for _, workspaceFilename := range workspaceFilenames {
			workspacePath := path.Join(cwd, workspaceFilename)
			_, err := f.osStat(workspacePath)

			if err == nil {
				return workspacePath, nil
			}
			if !os.IsNotExist(err) {
				return "", fmt.Errorf("failed to find bazel workspace: %w", err)
			}
			cwd = filepath.Dir(cwd)
		}
	}
}
