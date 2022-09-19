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

package bazel_test

import (
	"io"
	"os"
	"strings"
	"testing"

	"aspect.build/cli/buildinfo"
	"aspect.build/cli/pkg/bazel"
	. "github.com/onsi/gomega"
)

func TestVersionPath(t *testing.T) {
	g := NewWithT(t)
	actual := bazel.VersionPath("/path/to/workspace")
	g.Expect(actual).To(Equal("/path/to/workspace/.bazelversion"))
}

func TestNewVersion(t *testing.T) {
	g := NewWithT(t)
	actual := bazel.NewVersion()
	expected := &bazel.Version{
		Bazel:  "",
		Aspect: buildinfo.PreStampRelease,
	}
	g.Expect(actual).To(Equal(expected))
}

func TestNewVersionFromReader(t *testing.T) {
	t.Run("Bazel version, Aspect version", func(t *testing.T) {
		g := NewWithT(t)
		input := `
aspect-build/0.6.0
5.3.0
`
		actual, err := bazel.NewVersionFromReader(strings.NewReader(input))
		g.Expect(err).ToNot(HaveOccurred())
		expected := &bazel.Version{
			Bazel:  "5.3.0",
			Aspect: "0.6.0",
		}
		g.Expect(actual).To(Equal(expected))
	})
	t.Run("Bazel version, no Aspect version", func(t *testing.T) {
		g := NewWithT(t)
		input := `
5.3.0
`
		actual, err := bazel.NewVersionFromReader(strings.NewReader(input))
		g.Expect(err).ToNot(HaveOccurred())
		expected := &bazel.Version{
			Bazel:  "5.3.0",
			Aspect: "",
		}
		g.Expect(actual).To(Equal(expected))
	})
	t.Run("no Bazel version, Aspect version", func(t *testing.T) {
		g := NewWithT(t)
		input := `
aspect-build/0.6.0
`
		actual, err := bazel.NewVersionFromReader(strings.NewReader(input))
		g.Expect(err).ToNot(HaveOccurred())
		expected := &bazel.Version{
			Bazel:  "",
			Aspect: "0.6.0",
		}
		g.Expect(actual).To(Equal(expected))
	})
	t.Run("no Bazel version, no Aspect version", func(t *testing.T) {
		g := NewWithT(t)
		input := ""
		actual, err := bazel.NewVersionFromReader(strings.NewReader(input))
		g.Expect(err).ToNot(HaveOccurred())
		expected := &bazel.Version{
			Bazel:  "",
			Aspect: buildinfo.PreStampRelease,
		}
		g.Expect(actual).To(Equal(expected))
	})
	t.Run("with extraneous whitespace", func(t *testing.T) {
		g := NewWithT(t)
		input := `
   aspect-build/0.6.0
5.3.0   
`
		actual, err := bazel.NewVersionFromReader(strings.NewReader(input))
		g.Expect(err).ToNot(HaveOccurred())
		expected := &bazel.Version{
			Bazel:  "5.3.0",
			Aspect: "0.6.0",
		}
		g.Expect(actual).To(Equal(expected))
	})
}

func TestNewVersionFromFile(t *testing.T) {
	g := NewWithT(t)
	input := `
aspect-build/0.6.0
5.3.0
`
	f, err := os.CreateTemp("", ".bazelversion")
	defer f.Close()
	g.Expect(err).ToNot(HaveOccurred())
	_, err = io.WriteString(f, input)
	g.Expect(err).ToNot(HaveOccurred())
	err = f.Sync()
	g.Expect(err).ToNot(HaveOccurred())

	actual, err := bazel.NewVersionFromFile(f.Name())
	g.Expect(err).ToNot(HaveOccurred())
	expected := &bazel.Version{
		Bazel:  "5.3.0",
		Aspect: "0.6.0",
	}
	g.Expect(actual).To(Equal(expected))
}

func TestSafeVersionFromFile(t *testing.T) {
	t.Run("when .bazelversion exists", func(t *testing.T) {
		g := NewWithT(t)
		input := `
aspect-build/0.6.0
5.3.0
`
		f, err := os.CreateTemp("", ".bazelversion")
		defer f.Close()
		g.Expect(err).ToNot(HaveOccurred())
		_, err = io.WriteString(f, input)
		g.Expect(err).ToNot(HaveOccurred())
		err = f.Sync()
		g.Expect(err).ToNot(HaveOccurred())

		actual, err := bazel.SafeVersionFromFile(f.Name())
		g.Expect(err).ToNot(HaveOccurred())
		expected := &bazel.Version{
			Bazel:  "5.3.0",
			Aspect: "0.6.0",
		}
		g.Expect(actual).To(Equal(expected))
	})
	t.Run("when .bazelversion does not exist", func(t *testing.T) {
		g := NewWithT(t)
		wr, err := os.MkdirTemp("", "wksp_root")
		g.Expect(err).ToNot(HaveOccurred())

		path := bazel.VersionPath(wr)
		actual, err := bazel.SafeVersionFromFile(path)
		g.Expect(err).ToNot(HaveOccurred())
		expected := &bazel.Version{
			Bazel:  "",
			Aspect: buildinfo.PreStampRelease,
		}
		g.Expect(actual).To(Equal(expected))
	})
}
