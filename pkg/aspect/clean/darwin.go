//go:build darwin

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
	"io/ioutil"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

func (c *Clean) GetAccessTime(workspace fs.FileInfo) time.Duration {
	accessTime := workspace.Sys().(*syscall.Stat_t).Atimespec
	createdTime := workspace.Sys().(*syscall.Stat_t).Ctimespec
	modifiedTime := workspace.Sys().(*syscall.Stat_t).Mtimespec

	timeSinceAccess := time.Since(time.Unix(accessTime.Sec, accessTime.Nsec))
	timeSinceCreation := time.Since(time.Unix(createdTime.Sec, createdTime.Nsec))
	timeSinceModified := time.Since(time.Unix(modifiedTime.Sec, modifiedTime.Nsec))

	smallestTime := timeSinceAccess

	if timeSinceCreation < timeSinceAccess && timeSinceCreation < timeSinceModified {
		smallestTime = timeSinceCreation
	} else if timeSinceModified < timeSinceAccess && timeSinceModified < timeSinceCreation {
		smallestTime = timeSinceModified
	}

	return smallestTime
}

func (c *Clean) MoveDirectoryToTmp(dir string, name string) string {
	tempDir, err := ioutil.TempDir("", "aspect_delete")
	if err != nil {
		return ""
	}
	newDirectory := filepath.Join(tempDir + strings.Replace(dir, "/", "", -1))
	newPath := filepath.Join(newDirectory, name)
	os.MkdirAll(newDirectory, os.ModePerm)
	os.Rename(filepath.Join(dir, "external", name), newPath)

	return newPath
}

func (c *Clean) ChangeDirectoryPermissions(directory string) ([]byte, error) {
	cmd := exec.Command("chmod", "-R", "777", directory)
	return cmd.Output()
}
