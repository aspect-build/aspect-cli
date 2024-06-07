/* Copyright 2016 The Bazel Authors. All rights reserved.
Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at
   http://www.apache.org/licenses/LICENSE-2.0
Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

// NOTE: synced from bazel-gazelle/internal/wspace/finder.go

// Package wspace provides functions to locate and modify a bazel WORKSPACE file.
package wspace

import (
	"os"
	"path/filepath"
)

var workspaceFiles = []string{"WORKSPACE.bazel", "WORKSPACE"}

// IsWORKSPACE checks whether path is named WORKSPACE or WORKSPACE.bazel
func IsWORKSPACE(path string) bool {
	base := filepath.Base(path)
	for _, workspaceFile := range workspaceFiles {
		if base == workspaceFile {
			return true
		}
	}
	return false
}

// FindWORKSPACEFile returns a path to a file in the provided root directory,
// either to an existing WORKSPACE or WORKSPACE.bazel file, or to root/WORKSPACE
// if neither exists. Note that this function does NOT recursively check parent directories.
func FindWORKSPACEFile(root string) string {
	for _, workspaceFile := range workspaceFiles {
		path := filepath.Join(root, workspaceFile)
		if fileInfo, err := os.Stat(path); err == nil && !fileInfo.IsDir() {
			return path
		}
	}
	return filepath.Join(root, "WORKSPACE")
}
