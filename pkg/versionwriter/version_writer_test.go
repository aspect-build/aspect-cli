package versionwriter_test

import (
	"bytes"
	"testing"

	"aspect.build/cli/buildinfo"
	"aspect.build/cli/pkg/versionwriter"
	"github.com/stretchr/testify/assert"
)

const (
	name    = "Chicken"
	version = "1.2.3"
	format  = versionwriter.Conventional

	buildTime = "build time"
	hostName  = "host"
	gitCommit = "git commit"
	gitStatus = "git status"
	release   = "1.2.3"
)

func TestNew(t *testing.T) {
	actual := versionwriter.New(name, version, format)
	expected := &versionwriter.VersionWriter{
		Name:    name,
		Version: version,
		Format:  format,
	}
	assert.Equal(t, expected, actual)
}

func TestNewFromBuildInfo(t *testing.T) {
	t.Run("with release, is clean", func(t *testing.T) {
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release)
		actual := versionwriter.NewFromBuildInfo(name, *bi, format)
		assert.Equal(t, name, actual.Name)
		assert.Equal(t, format, actual.Format)
		assert.Equal(t, bi.Release, actual.Version)
	})
	t.Run("with release, is not clean", func(t *testing.T) {
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release)
		actual := versionwriter.NewFromBuildInfo(name, *bi, format)
		assert.Equal(t, name, actual.Name)
		assert.Equal(t, format, actual.Format)
		assert.Equal(t, bi.Release+versionwriter.NotCleanVersionSuffix, actual.Version)
	})
	t.Run("without release", func(t *testing.T) {
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, "")
		actual := versionwriter.NewFromBuildInfo(name, *bi, format)
		assert.Equal(t, name, actual.Name)
		assert.Equal(t, format, actual.Format)
		assert.Equal(t, versionwriter.NoReleaseVersion, actual.Version)
	})
}

func TestString(t *testing.T) {
	t.Run("with conventional format", func(t *testing.T) {
		vw := versionwriter.New(name, version, format)
		actual := vw.String()
		assert.Equal(t, "Chicken version: 1.2.3", actual)
	})
	t.Run("with GNU format", func(t *testing.T) {
		vw := versionwriter.New(name, version, versionwriter.GNU)
		actual := vw.String()
		assert.Equal(t, "Chicken 1.2.3", actual)
	})
}

func TestPrint(t *testing.T) {
	vw := versionwriter.New("Chicken", "1.2.3", versionwriter.Conventional)

	var buffer bytes.Buffer
	written, err := vw.Print(&buffer)
	if err != nil {
		t.Errorf("error from Print(): %s", err)
		return
	}
	assert.True(t, written > 0)
	output := buffer.String()
	assert.Equal(t, "Chicken version: 1.2.3\n", output)
}
