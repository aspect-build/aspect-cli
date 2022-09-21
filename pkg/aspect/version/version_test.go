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

package version_test

import (
	"strings"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"

	"aspect.build/cli/buildinfo"
	"aspect.build/cli/pkg/aspect/version"
	bazel_mock "aspect.build/cli/pkg/bazel/mock"
	"aspect.build/cli/pkg/ioutils"
)

const (
	buildTime      = "build time"
	hostName       = "host"
	gitCommit      = "git commit"
	dirtyGitStatus = ""
	release        = "1.2.3"
)

func TestVersion(t *testing.T) {
	t.Run("with a Bazel instance", func(t *testing.T) {
		g := NewWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		bzl := bazel_mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			RunCommand([]string{"version"}, streams).
			Return(0, nil)

		v := version.New(streams)
		err := v.Run(bzl)
		g.Expect(err).To(BeNil())
	})

	t.Run("with a Bazel instance, with --gnu_format", func(t *testing.T) {
		g := NewWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		bzl := bazel_mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			RunCommand([]string{"version", "--gnu_format"}, streams).
			Return(0, nil)

		v := version.New(streams)
		v.GNUFormat = true
		err := v.Run(bzl)
		g.Expect(err).To(BeNil())
	})

	t.Run("no Bazel instance, without release build info", func(t *testing.T) {
		g := NewGomegaWithT(t)
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		v := version.New(streams)
		err := v.Run(nil)
		g.Expect(err).To(BeNil())
		g.Expect(stdout.String()).To(Equal("Aspect version: unknown [not built with --stamp]\n"))
	})

	t.Run("no Bazel instance, with release build info", func(t *testing.T) {
		t.Run("git is clean", func(t *testing.T) {
			g := NewGomegaWithT(t)
			var stdout strings.Builder
			streams := ioutils.Streams{Stdout: &stdout}
			v := version.New(streams)
			v.BuildInfo = *buildinfo.New(
				buildTime,
				hostName,
				gitCommit,
				buildinfo.CleanGitStatus,
				release,
			)
			err := v.Run(nil)
			g.Expect(err).To(BeNil())
			g.Expect(stdout.String()).To(Equal("Aspect version: 1.2.3\n"))
		})

		t.Run("git is dirty", func(t *testing.T) {
			g := NewGomegaWithT(t)
			var stdout strings.Builder
			streams := ioutils.Streams{Stdout: &stdout}
			v := version.New(streams)
			v.BuildInfo = *buildinfo.New(
				buildTime,
				hostName,
				gitCommit,
				dirtyGitStatus,
				release,
			)
			err := v.Run(nil)
			g.Expect(err).To(BeNil())
			g.Expect(stdout.String()).To(Equal("Aspect version: 1.2.3 (with local changes)\n"))
		})
	})

	t.Run("no Bazel instance, with --gnu_format", func(t *testing.T) {
		g := NewGomegaWithT(t)
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		v := version.New(streams)
		v.GNUFormat = true
		v.BuildInfo = *buildinfo.New(
			buildTime,
			hostName,
			gitCommit,
			buildinfo.CleanGitStatus,
			release,
		)
		err := v.Run(nil)
		g.Expect(err).To(BeNil())
		g.Expect(stdout.String()).To(Equal("Aspect 1.2.3\n"))
	})
}
