/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package workspace

import (
	"io/fs"
	"os"
	"path"
	"path/filepath"
)

// https://github.com/bazelbuild/bazel/blob/8346ea4c/src/main/cpp/workspace_layout.cc#L37
var validWorkspaceFiles = []string{"WORKSPACE", "WORKSPACE.bazel"}

// Finder wraps the Find method that performs the finding of the WORKSPACE file
// in the user's Bazel project.
type Finder interface {
	Find(string) (string, error)
}

type finder struct {
	osStat func(string) (fs.FileInfo, error)
}

// DefaultFinder is the Finder with default dependencies.
var DefaultFinder = &finder{
	osStat: os.Stat,
}

// Find tries to find the root of a Bazel workspace.
func (f *finder) Find(startDir string) (string, error) {
	for maybeDir := startDir; maybeDir != "." && maybeDir != filepath.Dir(maybeDir); maybeDir = filepath.Dir(maybeDir) {
		for _, wf := range validWorkspaceFiles {
			maybeWorkspaceFile := path.Join(maybeDir, wf)
			stat, err := f.osStat(maybeWorkspaceFile)
			if err != nil {
				if os.IsNotExist(err) {
					continue
				}
				return "", err
			}
			if stat.IsDir() {
				continue
			}
			return path.Dir(maybeWorkspaceFile), nil
		}
	}

	return "", &NotFoundError{StartDir: startDir}
}
