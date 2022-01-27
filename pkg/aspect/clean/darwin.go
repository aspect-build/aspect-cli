//go:build darwin

package clean

import (
	"io/fs"
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
