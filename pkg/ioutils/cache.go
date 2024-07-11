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

package ioutils

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/mitchellh/go-homedir"
)

func UserCacheDir() (string, error) {
	userCacheDir, err := os.UserCacheDir()
	if err != nil {
		return "", fmt.Errorf("could not get the user's cache directory: %v", err)
	}

	// We hit a case in a bazel-in-bazel test where os.UserCacheDir() return a path starting with '~'.
	// Run it through homedir.Expand to turn it into an absolute path incase that happens.
	userCacheDir, err = homedir.Expand(userCacheDir)
	if err != nil {
		return "", fmt.Errorf("could not expand home directory in path: %v", err)
	}

	return userCacheDir, err
}

func AspectCacheDir() (string, error) {
	userCacheDir, err := UserCacheDir()
	if err != nil {
		return "", err
	}

	aspectCacheDir := filepath.Join(userCacheDir, "aspect")
	err = ensureDirectoryAtPath(aspectCacheDir)
	if err != nil {
		return "", err
	}
	return aspectCacheDir, nil
}

func ensureDirectoryAtPath(path string) error {
	// Get file info
	fileInfo, err := os.Stat(path)

	if err == nil {
		// Path exists
		if fileInfo.IsDir() {
			// It's already a directory, nothing to do
			return nil
		} else {
			// It's a file, delete it
			err = os.Remove(path)
			if err != nil {
				return err
			}
		}
	} else if !os.IsNotExist(err) {
		// An error occurred that wasn't "file not found"
		return err
	}

	// Create the directory
	err = os.MkdirAll(path, 0755)
	if err != nil {
		return err
	}

	return nil
}
