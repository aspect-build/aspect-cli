/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

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

	name    = "Chicken"
	version = "1.2.3"
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

func TestVersion(t *testing.T) {
	t.Run("with release, is clean", func(t *testing.T) {
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release)
		actual := bi.Version()
		assert.Equal(t, bi.Release, actual)
	})
	t.Run("with release, is not clean", func(t *testing.T) {
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release)
		actual := bi.Version()
		assert.Equal(t, bi.Release+buildinfo.NotCleanVersionSuffix, actual)
	})
	t.Run("without release", func(t *testing.T) {
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, "")
		actual := bi.Version()
		assert.Equal(t, buildinfo.NoReleaseVersion, actual)
	})
}

func TestUtilityVersion(t *testing.T) {
	t.Run("with conventional format", func(t *testing.T) {
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release)
		actual := bi.UtilityVersion(name, buildinfo.ConventionalFormat)
		assert.Equal(t, "Chicken version: 1.2.3", actual)
	})
	t.Run("with GNU format", func(t *testing.T) {
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release)
		actual := bi.UtilityVersion(name, buildinfo.GNUFormat)
		assert.Equal(t, "Chicken 1.2.3", actual)
	})
}
