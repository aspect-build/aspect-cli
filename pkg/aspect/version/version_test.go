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
	"context"
	"strings"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
	"github.com/spf13/cobra"

	"github.com/aspect-build/aspect-cli/buildinfo"
	"github.com/aspect-build/aspect-cli/pkg/aspect/version"
	bazel_mock "github.com/aspect-build/aspect-cli/pkg/bazel/mock"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
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
			RunCommand(streams, nil, "version").
			Return(nil)

		v := version.New(streams, bzl)
		v.BuildInfo = *buildinfo.New(
			buildTime,
			hostName,
			gitCommit,
			buildinfo.CleanGitStatus,
			release,
		)
		cmd := &cobra.Command{}
		cmd.Flags().Bool("gnu_format", false, "")
		err := v.Run(context.Background(), cmd, []string{})
		g.Expect(err).To(BeNil())
		g.Expect(stdout.String()).To(Equal("Aspect CLI version: 1.2.3\n"))
	})

	t.Run("with --gnu_format", func(t *testing.T) {
		g := NewWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		bzl := bazel_mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			RunCommand(streams, nil, "version", "--gnu_format").
			Return(nil)

		v := version.New(streams, bzl)
		v.BuildInfo = *buildinfo.New(
			buildTime,
			hostName,
			gitCommit,
			buildinfo.CleanGitStatus,
			release,
		)
		cmd := &cobra.Command{}
		gnuFormat := cmd.Flags().Bool("gnu_format", false, "")
		*gnuFormat = true
		err := v.Run(context.Background(), cmd, []string{"--gnu_format"})
		g.Expect(err).To(BeNil())
		g.Expect(stdout.String()).To(Equal("aspect 1.2.3\n"))
	})

	t.Run("git is clean", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		bzl := bazel_mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			RunCommand(streams, nil, "version").
			Return(nil)
		v := version.New(streams, bzl)
		v.BuildInfo = *buildinfo.New(
			buildTime,
			hostName,
			gitCommit,
			buildinfo.CleanGitStatus,
			release,
		)
		cmd := &cobra.Command{}
		cmd.Flags().Bool("gnu_format", false, "")
		err := v.Run(context.Background(), cmd, []string{})
		g.Expect(err).To(BeNil())
		g.Expect(stdout.String()).To(Equal("Aspect CLI version: 1.2.3\n"))
	})

	t.Run("git is dirty", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		bzl := bazel_mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			RunCommand(streams, nil, "version").
			Return(nil)
		v := version.New(streams, bzl)
		v.BuildInfo = *buildinfo.New(
			buildTime,
			hostName,
			gitCommit,
			dirtyGitStatus,
			release,
		)
		cmd := &cobra.Command{}
		cmd.Flags().Bool("gnu_format", false, "")
		err := v.Run(context.Background(), cmd, []string{})
		g.Expect(err).To(BeNil())
		g.Expect(stdout.String()).To(Equal("Aspect CLI version: 1.2.3 (with local changes)\n"))
	})
}
