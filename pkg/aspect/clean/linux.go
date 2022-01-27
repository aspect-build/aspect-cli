//go:build linux

package clean

import (
	"io/fs"
	"syscall"
	"time"
)

func (c *Clean) GetAccessTime(workspace fs.FileInfo) time.Duration {
	accessTime := workspace.Sys().(*syscall.Stat_t).Atim
	createdTime := workspace.Sys().(*syscall.Stat_t).Ctim
	modifiedTime := workspace.Sys().(*syscall.Stat_t).Mtim

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
