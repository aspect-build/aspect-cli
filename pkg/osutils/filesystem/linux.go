//go:build linux

/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package filesystem

import (
	"io/fs"
	"io/ioutil"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

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
	tempDir, err := ioutil.TempDir("", "aspect_delete")
	if err != nil {
		return "", nil
	}
	newDirectory := filepath.Join(tempDir + strings.Replace(dir, "/", "", -1))
	newPath := filepath.Join(newDirectory, name)

	err = os.MkdirAll(newDirectory, os.ModePerm)
	if err != nil {
		return "", err
	}

	err = os.Rename(filepath.Join(dir, "external", name), newPath)
	if err != nil {
		return "", err
	}

	return newPath, nil
}

func (f *Filesystem) changeDirectoryPermissions(directory string, permissions string) ([]byte, error) {
	cmd := exec.Command("chmod", "-R", "777", directory)
	return cmd.Output()
}
