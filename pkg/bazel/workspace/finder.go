/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package workspace

import (
	"fmt"
	"io/fs"
	"os"
	"path"
	"path/filepath"
)

// https://github.com/bazelbuild/bazel/blob/8346ea4c/src/main/cpp/workspace_layout.cc#L37
var workspaceFilenames = []string{"WORKSPACE", "WORKSPACE.bazel"}

// Finder wraps the Find method that performs the finding of the WORKSPACE file
// in the user's Bazel project.
type Finder interface {
	Find() (string, error)
}

type finder struct {
	osGetwd func() (dir string, err error)
	osStat  func(string) (fs.FileInfo, error)

	workspaceRoot string
}

// DefaultFinder is the Finder with default dependencies.
var DefaultFinder = &finder{
	osGetwd: os.Getwd,
	osStat:  os.Stat,
}

// Find tries to find the root of a Bazel workspace.
func (f *finder) Find() (string, error) {
	if f.workspaceRoot != "" {
		return f.workspaceRoot, nil
	}

	wd, err := f.osGetwd()
	if err != nil {
		return "", fmt.Errorf("failed to find bazel workspace: %w", err)
	}

	for current := wd; current != "." && current != filepath.Dir(current); current = filepath.Dir(current) {
		for _, workspaceFilename := range workspaceFilenames {
			workspacePath := path.Join(current, workspaceFilename)
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
			f.workspaceRoot = path.Dir(workspacePath)
			return f.workspaceRoot, nil
		}
	}

	return "", fmt.Errorf("failed to find bazel workspace: the current working directory %q is not a Bazel workspace", wd)
}
