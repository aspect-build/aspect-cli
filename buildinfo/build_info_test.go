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
	. "github.com/onsi/gomega"
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
	g := NewGomegaWithT(t)
	actual := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release)
	expected := &buildinfo.BuildInfo{
		BuildTime: buildTime,
		HostName:  hostName,
		GitCommit: gitCommit,
		GitStatus: gitStatus,
		Release:   release,
	}
	g.Expect(actual).To(Equal(expected))
}

func TestCurrent(t *testing.T) {
	g := NewGomegaWithT(t)
	actual := buildinfo.Current()
	expected := &buildinfo.BuildInfo{
		BuildTime: buildinfo.BuildTime,
		HostName:  buildinfo.HostName,
		GitCommit: buildinfo.GitCommit,
		GitStatus: buildinfo.GitStatus,
		Release:   buildinfo.Release,
	}
	g.Expect(actual).To(Equal(expected))
}

func TestBuildinfoHasRelease(t *testing.T) {
	t.Run("has a release value", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release)
		g.Expect(bi.HasRelease()).To(BeTrue())
	})
	t.Run("does not have a release value", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, "")
		g.Expect(bi.HasRelease()).To(BeFalse())
	})
	t.Run("has pre-stamp release value", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, buildinfo.PreStampRelease)
		g.Expect(bi.HasRelease()).To(BeFalse())
	})
}

func TestBuildinfoIsClean(t *testing.T) {
	t.Run("has a clean git status", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release)
		g.Expect(bi.IsClean()).To(BeTrue())
	})
	t.Run("does not have a clean git status", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release)
		g.Expect(bi.IsClean()).To(BeFalse())
	})
}

func TestVersion(t *testing.T) {
	t.Run("with release, is clean", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release)
		actual := bi.Version()
		g.Expect(actual).To(Equal(bi.Release))
	})
	t.Run("with release, is not clean", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release)
		actual := bi.Version()
		g.Expect(actual).To(Equal(bi.Release + buildinfo.NotCleanVersionSuffix))
	})
	t.Run("without release", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, "")
		actual := bi.Version()
		g.Expect(actual).To(Equal(buildinfo.NoReleaseVersion))
	})
}

func TestCommandVersion(t *testing.T) {
	t.Run("with conventional format", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release)
		actual := bi.CommandVersion(name, buildinfo.ConventionalFormat)
		g.Expect(actual).To(Equal("Chicken version: 1.2.3"))
	})
	t.Run("with GNU format", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release)
		actual := bi.CommandVersion(name, buildinfo.GNUFormat)
		g.Expect(actual).To(Equal("Chicken 1.2.3"))
	})
}
