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
)

// https://github.com/bazelbuild/bazel/blob/8346ea4c/src/main/cpp/workspace_layout.cc#L37
var WorkspaceFilenames = []string{"WORKSPACE", "WORKSPACE.bazel"}

// WorkspaceFinder wraps the Find method that performs the finding of the
// WORKSPACE file in the user's Bazel project.
type WorkspaceFinder interface {
	Find(wd string) (string, error)
}

type workspaceFinder struct {
	osStat func(string) (fs.FileInfo, error)
}

var DefaultWorkspaceFinder = &workspaceFinder{osStat: os.Stat}

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
