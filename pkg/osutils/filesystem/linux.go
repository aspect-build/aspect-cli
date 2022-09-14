//go:build linux

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

package filesystem

import (
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

const ChmodPath = "chmod"

func (f *Filesystem) getAccessTime(workspace fs.FileInfo) time.Duration {
	accessTime := workspace.Sys().(*syscall.Stat_t).Atim
	createdTime := workspace.Sys().(*syscall.Stat_t).Ctim
	modifiedTime := workspace.Sys().(*syscall.Stat_t).Mtim

	timeSinceAccess := f.TimeSince(f.TimeUnix(accessTime.Sec, accessTime.Nsec))
	timeSinceCreation := f.TimeSince(f.TimeUnix(createdTime.Sec, createdTime.Nsec))
	timeSinceModified := f.TimeSince(f.TimeUnix(modifiedTime.Sec, modifiedTime.Nsec))

	smallestTime := timeSinceAccess

	if timeSinceCreation < timeSinceAccess && timeSinceCreation < timeSinceModified {
		smallestTime = timeSinceCreation
	} else if timeSinceModified < timeSinceAccess && timeSinceModified < timeSinceCreation {
		smallestTime = timeSinceModified
	}

	return smallestTime
}

func (f *Filesystem) moveDirectoryToTmp(dir string, name string) (string, error) {
	tempDir, err := f.IoutilTempDir("", "aspect_delete")
	if err != nil {
		return "", nil
	}
	newDirectory := filepath.Join(tempDir + strings.Replace(dir, "/", "", -1))
	newPath := filepath.Join(newDirectory, name)

	err = f.OsMkdirAll(newDirectory, os.ModePerm)
	if err != nil {
		return "", err
	}

	err = f.OsRename(filepath.Join(dir, "external", name), newPath)
	if err != nil {
		return "", err
	}

	return newPath, nil
}

func (f *Filesystem) changeDirectoryPermissions(directory string, permissions string) ([]byte, error) {
	if _, err := f.OsStat(ChmodPath); errors.Is(err, os.ErrNotExist) {
		panic(fmt.Errorf("%q does not exist", ChmodPath))
	}
	cmd := f.OsExecCommand(ChmodPath, "-R", permissions, directory)
	return cmd.Output()
}
