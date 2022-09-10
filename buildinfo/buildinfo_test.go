package buildinfo_test

import (
	"testing"

	"aspect.build/cli/buildinfo"
	"github.com/stretchr/testify/assert"
)

func TestBuildinfoHasRelease(t *testing.T) {
	t.Cleanup(resetState)
	t.Run("has a release value", func(t *testing.T) {
		buildinfo.Release = "1.2.3"
		assert.True(t, buildinfo.HasRelease())
	})
	t.Run("does not have a release value", func(t *testing.T) {
		buildinfo.Release = ""
		assert.False(t, buildinfo.HasRelease())
	})
}

func TestBuildinfoIsClean(t *testing.T) {
	t.Cleanup(resetState)
	t.Run("has a clean git status", func(t *testing.T) {
		buildinfo.GitStatus = buildinfo.CleanGitStatus
		assert.True(t, buildinfo.IsClean())
	})
	t.Run("does not have a clean git status", func(t *testing.T) {
		buildinfo.GitStatus = "very dirty"
		assert.False(t, buildinfo.IsClean())
	})
}

func resetState() {
	buildinfo.Release = buildinfo.DefaultRelease
	buildinfo.GitStatus = buildinfo.DefaultGitStatus
}
