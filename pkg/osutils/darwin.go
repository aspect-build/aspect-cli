//go:build darwin

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
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

func (o *OsUtils) getAccessTime(workspace fs.FileInfo) time.Duration {
	accessTime := workspace.Sys().(*syscall.Stat_t).Atimespec
	createdTime := workspace.Sys().(*syscall.Stat_t).Ctimespec
	modifiedTime := workspace.Sys().(*syscall.Stat_t).Mtimespec

	timeSinceAccess := o.TimeSince(o.TimeUnix(accessTime.Sec, accessTime.Nsec))
	timeSinceCreation := o.TimeSince(o.TimeUnix(createdTime.Sec, createdTime.Nsec))
	timeSinceModified := o.TimeSince(o.TimeUnix(modifiedTime.Sec, modifiedTime.Nsec))

	smallestTime := timeSinceAccess

	if timeSinceCreation < timeSinceAccess && timeSinceCreation < timeSinceModified {
		smallestTime = timeSinceCreation
	} else if timeSinceModified < timeSinceAccess && timeSinceModified < timeSinceCreation {
		smallestTime = timeSinceModified
	}

	return smallestTime
}

func (o *OsUtils) moveDirectoryToTmp(dir string, name string) string {
	tempDir, err := o.IoutilTempDir("", "aspect_delete")
	if err != nil {
		return ""
	}
	newDirectory := filepath.Join(tempDir + strings.Replace(dir, "/", "", -1))
	newPath := filepath.Join(newDirectory, name)
	o.OsMkdirAll(newDirectory, os.ModePerm)
	o.OsRename(filepath.Join(dir, "external", name), newPath)

	return newPath
}

func (o *OsUtils) changeDirectoryPermissions(directory string) ([]byte, error) {
	cmd := o.OsExecCommand("chmod", "-R", "777", directory)
	return cmd.Output()
}
