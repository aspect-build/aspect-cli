//go:build windows

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
