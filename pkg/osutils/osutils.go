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
	"io/ioutil"
	"os"
	"os/exec"
	"time"
)

func timeSince(t time.Time) time.Duration {
	return time.Since(t)
}

func timeUnix(sec int64, nsec int64) time.Time {
	return time.Unix(sec, nsec)
}

func ioutilTempDir(dir string, pattern string) (name string, err error) {
	return ioutil.TempDir(dir, pattern)
}

func osMkdirAll(path string, perm fs.FileMode) error {
	return os.MkdirAll(path, perm)
}

func osRename(oldpath string, newpath string) error {
	return os.Rename(oldpath, newpath)
}

func osExecCommand(name string, arg ...string) *exec.Cmd {
	return exec.Command(name, arg...)
}

type OsUtils struct {
	TimeSince     func(time.Time) time.Duration
	TimeUnix      func(int64, int64) time.Time
	IoutilTempDir func(string, string) (name string, err error)
	OsMkdirAll    func(string, fs.FileMode) error
	OsRename      func(string, string) error
	OsExecCommand func(string, ...string) *exec.Cmd
}

func NewDefault() OsUtils {
	osUtils := OsUtils{}
	osUtils.TimeSince = timeSince
	osUtils.TimeUnix = timeUnix
	osUtils.IoutilTempDir = ioutilTempDir
	osUtils.OsMkdirAll = osMkdirAll
	osUtils.OsRename = osRename
	osUtils.OsExecCommand = osExecCommand
	return osUtils
}

func (os *OsUtils) GetAccessTime(workspace fs.FileInfo) time.Duration {
	return os.getAccessTime(workspace)
}

func (os *OsUtils) MoveDirectoryToTmp(dir string, name string) string {
	return os.moveDirectoryToTmp(dir, name)
}

func (os *OsUtils) ChangeDirectoryPermissions(directory string) ([]byte, error) {
	return os.changeDirectoryPermissions(directory)
}
