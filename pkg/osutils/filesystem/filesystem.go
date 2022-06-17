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

// type ConfirmationRunner interface {
// 	Run() (string, error)
// }

// func Confirmation(question string) ConfirmationRunner {
// 	return &promptui.Prompt{
// 		Label:     question,
// 		IsConfirm: true,
// 	}
// }

type ExecCmdRunner interface {
	Output() ([]byte, error)

	// func (*exec.Cmd).Output() ([]byte, error)
}

// func ExecCmd(question string) ExecCmdRunner {
// 	return &promptui.Prompt{
// 		Label:     question,
// 		IsConfirm: true,
// 	}
// }

func osExecCommand(name string, arg ...string) ExecCmdRunner {
	return exec.Command(name, arg...)
}

func osStat(name string) (fs.FileInfo, error) {
	return os.Stat(name)
}

type Filesystem struct {
	TimeSince     func(time.Time) time.Duration
	TimeUnix      func(int64, int64) time.Time
	IoutilTempDir func(string, string) (name string, err error)
	OsMkdirAll    func(string, fs.FileMode) error
	OsRename      func(string, string) error
	OsExecCommand func(string, ...string) ExecCmdRunner
	OsStat        func(string) (fs.FileInfo, error)
}

func NewDefault() Filesystem {
	osUtils := Filesystem{}
	osUtils.TimeSince = timeSince
	osUtils.TimeUnix = timeUnix
	osUtils.IoutilTempDir = ioutilTempDir
	osUtils.OsMkdirAll = osMkdirAll
	osUtils.OsRename = osRename
	osUtils.OsExecCommand = osExecCommand
	osUtils.OsStat = osStat
	return osUtils
}

func (f *Filesystem) GetAccessTime(workspace fs.FileInfo) time.Duration {
	return f.getAccessTime(workspace)
}

func (f *Filesystem) MoveDirectoryToTmp(dir string, name string) string {
	return f.moveDirectoryToTmp(dir, name)
}

func (f *Filesystem) ChangeDirectoryPermissions(directory string, permissions string) ([]byte, error) {
	return f.changeDirectoryPermissions(directory, permissions)
}
