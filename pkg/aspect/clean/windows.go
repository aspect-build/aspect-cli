//go:build windows

/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package clean

import (
	"io/fs"
	"syscall"
	"time"
)

func (c *Clean) GetAccessTime(workspace fs.FileInfo) time.Duration {
	winFileData := workspace.Sys().(*syscall.Win32FileAttributeData)

	timeSinceAccess := time.Since(time.Unix(0, winFileData.LastAccessTime.Nanoseconds()))
	timeSinceCreation := time.Since(time.Unix(0, winFileData.CreationTime.Nanoseconds()))
	timeSinceModified := time.Since(time.Unix(0, winFileData.LastWriteTime.Nanoseconds()))

	smallestTime := timeSinceAccess

	if timeSinceCreation < timeSinceAccess && timeSinceCreation < timeSinceModified {
		smallestTime = timeSinceCreation
	} else if timeSinceModified < timeSinceAccess && timeSinceModified < timeSinceCreation {
		smallestTime = timeSinceModified
	}

	return smallestTime
}

func (c *Clean) MoveFolderToTmp(dir string, name string) string {
	// TODO: Add functionality. https://github.com/aspect-build/aspect-cli/issues/196
	return ""
}

func (c *Clean) ChangeFolderPermissions(folder string) ([]byte, error) {
	// TODO: Add functionality. https://github.com/aspect-build/aspect-cli/issues/196
	return nil, nil
}
