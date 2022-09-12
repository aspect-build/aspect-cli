package buildinfo_test

import (
	"testing"

	"aspect.build/cli/buildinfo"
	"github.com/stretchr/testify/assert"
)

const (
	buildTime = "build time"
	hostName  = "host"
	gitCommit = "git commit"
	gitStatus = "git status"
	release   = "1.2.3"
)

func TestNew(t *testing.T) {
	actual := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release)
	expected := &buildinfo.BuildInfo{
		BuildTime: buildTime,
		HostName:  hostName,
		GitCommit: gitCommit,
		GitStatus: gitStatus,
		Release:   release,
	}
	assert.Equal(t, expected, actual)
}

func TestCurrent(t *testing.T) {
	actual := buildinfo.Current()
	expected := &buildinfo.BuildInfo{
		BuildTime: buildinfo.BuildTime,
		HostName:  buildinfo.HostName,
		GitCommit: buildinfo.GitCommit,
		GitStatus: buildinfo.GitStatus,
		Release:   buildinfo.Release,
	}
	assert.Equal(t, expected, actual)
}

func TestBuildinfoHasRelease(t *testing.T) {
	t.Run("has a release value", func(t *testing.T) {
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release)
		assert.True(t, bi.HasRelease())
	})
	t.Run("does not have a release value", func(t *testing.T) {
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, "")
		assert.False(t, bi.HasRelease())
	})
}

func TestBuildinfoIsClean(t *testing.T) {
	t.Run("has a clean git status", func(t *testing.T) {
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release)
		assert.True(t, bi.IsClean())
	})
	t.Run("does not have a clean git status", func(t *testing.T) {
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release)
		assert.False(t, bi.IsClean())
	})
}
