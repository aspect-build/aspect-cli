/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
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

	version = "1.2.3"
)

func TestNew(t *testing.T) {
	g := NewGomegaWithT(t)
	actual := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release, false)
	expected := &buildinfo.BuildInfo{
		BuildTime:   buildTime,
		HostName:    hostName,
		GitCommit:   gitCommit,
		GitStatus:   gitStatus,
		Release:     release,
		IsAspectPro: false,
	}
	g.Expect(actual).To(Equal(expected))
}

func TestCurrent(t *testing.T) {
	g := NewGomegaWithT(t)
	actual := buildinfo.Current()
	expected := &buildinfo.BuildInfo{
		BuildTime:   buildinfo.BuildTime,
		HostName:    buildinfo.HostName,
		GitCommit:   buildinfo.GitCommit,
		GitStatus:   buildinfo.GitStatus,
		Release:     buildinfo.Release,
		IsAspectPro: buildinfo.IsAspectPro != "",
	}
	g.Expect(actual).To(Equal(expected))
}

func TestBuildinfoHasRelease(t *testing.T) {
	t.Run("has a release value", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release, false)
		g.Expect(bi.HasRelease()).To(BeTrue())
	})
	t.Run("does not have a release value", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, "", false)
		g.Expect(bi.HasRelease()).To(BeFalse())
	})
	t.Run("has pre-stamp release value", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, buildinfo.PreStampRelease, false)
		g.Expect(bi.HasRelease()).To(BeFalse())
	})
}

func TestBuildinfoIsClean(t *testing.T) {
	t.Run("has a clean git status", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release, false)
		g.Expect(bi.IsClean()).To(BeTrue())
	})
	t.Run("does not have a clean git status", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release, false)
		g.Expect(bi.IsClean()).To(BeFalse())
	})
}

func TestVersion(t *testing.T) {
	t.Run("with release, is clean", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release, false)
		actual := bi.Version()
		g.Expect(actual).To(Equal(bi.Release))
	})
	t.Run("with release, is not clean", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, gitStatus, release, false)
		actual := bi.Version()
		g.Expect(actual).To(Equal(bi.Release + buildinfo.NotCleanVersionSuffix))
	})
	t.Run("without release", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, "", false)
		actual := bi.Version()
		g.Expect(actual).To(Equal(buildinfo.NoReleaseVersion))
	})
}

func TestCommandVersion(t *testing.T) {
	t.Run("with conventional format", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release, false)
		actual := bi.CommandVersion(buildinfo.ConventionalFormat)
		g.Expect(actual).To(Equal("Aspect CLI version: 1.2.3"))
	})
	t.Run("with GNU format", func(t *testing.T) {
		g := NewGomegaWithT(t)
		bi := buildinfo.New(buildTime, hostName, gitCommit, buildinfo.CleanGitStatus, release, false)
		actual := bi.CommandVersion(buildinfo.GNUFormat)
		g.Expect(actual).To(Equal("aspect 1.2.3"))
	})
}
