/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package version_test

import (
	"strings"
	"testing"

	. "github.com/onsi/gomega"

	"aspect.build/cli/pkg/aspect/version"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

func TestVersion(t *testing.T) {
	bzl := bazel.New()
	t.Run("without release build info", func(t *testing.T) {
		g := NewGomegaWithT(t)
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		v := version.New(streams)
		err := v.Run(bzl)
		g.Expect(err).To(BeNil())
		g.Expect(stdout.String()).To(Equal("Aspect version: unknown [not built with --stamp]\n"))
	})

	t.Run("with release build info", func(t *testing.T) {
		t.Run("git is clean", func(t *testing.T) {
			g := NewGomegaWithT(t)
			var stdout strings.Builder
			streams := ioutils.Streams{Stdout: &stdout}
			v := version.New(streams)
			v.BuildinfoRelease = "1.2.3"
			v.BuildinfoGitStatus = "clean"
			err := v.Run(bzl)
			g.Expect(err).To(BeNil())
			g.Expect(stdout.String()).To(Equal("Aspect version: 1.2.3\n"))
		})

		t.Run("git is dirty", func(t *testing.T) {
			g := NewGomegaWithT(t)
			var stdout strings.Builder
			streams := ioutils.Streams{Stdout: &stdout}
			v := version.New(streams)
			v.BuildinfoRelease = "1.2.3"
			v.BuildinfoGitStatus = ""
			err := v.Run(bzl)
			g.Expect(err).To(BeNil())
			g.Expect(stdout.String()).To(Equal("Aspect version: 1.2.3 (with local changes)\n"))
		})
	})

	t.Run("with --gnu_format", func(t *testing.T) {
		g := NewGomegaWithT(t)
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		v := version.New(streams)
		v.GNUFormat = true
		v.BuildinfoRelease = "1.2.3"
		v.BuildinfoGitStatus = "clean"
		err := v.Run(bzl)
		g.Expect(err).To(BeNil())
		g.Expect(stdout.String()).To(Equal("Aspect 1.2.3\n"))
	})
}
