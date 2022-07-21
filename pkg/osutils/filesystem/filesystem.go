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

// ExecCmdRunner is the interface that wraps exec.Command from the os package
type ExecCmdRunner interface {
	Output() ([]byte, error)
}

func osExecCommand(name string, arg ...string) ExecCmdRunner {
	return exec.Command(name, arg...)
}

func osStat(name string) (fs.FileInfo, error) {
	return os.Stat(name)
}

// Filesystem creates multiple signatures representing functions that would normally interact directly
// with the filesystem
type Filesystem struct {
	TimeSince     func(time.Time) time.Duration
	TimeUnix      func(int64, int64) time.Time
	IoutilTempDir func(string, string) (name string, err error)
	OsMkdirAll    func(string, fs.FileMode) error
	OsRename      func(string, string) error
	OsExecCommand func(string, ...string) ExecCmdRunner
	OsStat        func(string) (fs.FileInfo, error)
}

// NewDefault creates a new default Filesystem
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

// GetAccessTime finds the most recent time that a file was created, modified or accessed
func (f *Filesystem) GetAccessTime(workspace fs.FileInfo) time.Duration {
	return f.getAccessTime(workspace)
}

// MoveDirectoryToTmp will move a given directory to /tmp
func (f *Filesystem) MoveDirectoryToTmp(dir string, name string) (string, error) {
	return f.moveDirectoryToTmp(dir, name)
}

// ChangeDirectoryPermissions will update a directory to have the given permissions
func (f *Filesystem) ChangeDirectoryPermissions(directory string, permissions string) ([]byte, error) {
	return f.changeDirectoryPermissions(directory, permissions)
}
