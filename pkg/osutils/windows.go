//go:build windows

/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package osutils

import (
	"io/fs"
	"syscall"
	"time"
)

func (o *OsUtils) getAccessTime(workspace fs.FileInfo) time.Duration {
	winFileData := workspace.Sys().(*syscall.Win32FileAttributeData)

	timeSinceAccess := o.TimeSince(o.TimeUnix(0, winFileData.LastAccessTime.Nanoseconds()))
	timeSinceCreation := o.TimeSince(o.TimeUnix(0, winFileData.CreationTime.Nanoseconds()))
	timeSinceModified := o.TimeSince(o.TimeUnix(0, winFileData.LastWriteTime.Nanoseconds()))

	smallestTime := timeSinceAccess

	if timeSinceCreation < timeSinceAccess && timeSinceCreation < timeSinceModified {
		smallestTime = timeSinceCreation
	} else if timeSinceModified < timeSinceAccess && timeSinceModified < timeSinceCreation {
		smallestTime = timeSinceModified
	}

	return smallestTime
}

func (o *OsUtils) moveDirectoryToTmp(dir string, name string) string {
	// TODO: Add functionality. https://github.com/aspect-build/aspect-cli/issues/196
	return ""
}

func (o *OsUtils) changeDirectoryPermissions(directory string) ([]byte, error) {
	// TODO: Add functionality. https://github.com/aspect-build/aspect-cli/issues/196
	return nil, nil
}
