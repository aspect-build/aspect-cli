//go:build windows

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
	"io/fs"
	"syscall"
	"time"
)

func (f *Filesystem) getAccessTime(workspace fs.FileInfo) time.Duration {
	winFileData := workspace.Sys().(*syscall.Win32FileAttributeData)

	timeSinceAccess := f.TimeSince(f.TimeUnix(0, winFileData.LastAccessTime.Nanoseconds()))
	timeSinceCreation := f.TimeSince(f.TimeUnix(0, winFileData.CreationTime.Nanoseconds()))
	timeSinceModified := f.TimeSince(f.TimeUnix(0, winFileData.LastWriteTime.Nanoseconds()))

	smallestTime := timeSinceAccess

	if timeSinceCreation < timeSinceAccess && timeSinceCreation < timeSinceModified {
		smallestTime = timeSinceCreation
	} else if timeSinceModified < timeSinceAccess && timeSinceModified < timeSinceCreation {
		smallestTime = timeSinceModified
	}

	return smallestTime
}

func (f *Filesystem) moveDirectoryToTmp(dir string, name string) (string, error) {
	// TODO: Add functionality. https://github.com/aspect-build/aspect-cli/issues/196
	return "", nil
}

func (f *Filesystem) changeDirectoryPermissions(directory string, permissions string) ([]byte, error) {
	// TODO: Add functionality. https://github.com/aspect-build/aspect-cli/issues/196
	return nil, nil
}
