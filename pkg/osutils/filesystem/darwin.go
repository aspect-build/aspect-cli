//go:build darwin

/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
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

const ChmodPath = "/bin/chmod"

func (f *Filesystem) getAccessTime(workspace fs.FileInfo) time.Duration {
	accessTime := workspace.Sys().(*syscall.Stat_t).Atimespec
	createdTime := workspace.Sys().(*syscall.Stat_t).Ctimespec
	modifiedTime := workspace.Sys().(*syscall.Stat_t).Mtimespec

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

func (f *Filesystem) moveDirectoryToTmp(dir string, name string) string {
	tempDir, err := f.IoutilTempDir("", "aspect_delete")
	if err != nil {
		return ""
	}
	newDirectory := filepath.Join(tempDir + strings.Replace(dir, "/", "", -1))
	newPath := filepath.Join(newDirectory, name)
	f.OsMkdirAll(newDirectory, os.ModePerm)
	f.OsRename(filepath.Join(dir, "external", name), newPath)

	return newPath
}

func (f *Filesystem) changeDirectoryPermissions(directory string, permissions string) ([]byte, error) {
	if _, err := f.OsStat(ChmodPath); errors.Is(err, os.ErrNotExist) {
		panic(fmt.Errorf("%q does not exist", ChmodPath))
	}
	cmd := f.OsExecCommand(ChmodPath, "-R", permissions, directory)
	return cmd.Output()
}
