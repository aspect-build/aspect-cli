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
	"bytes"
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
	defer func() {
		f.Close()
		os.RemoveAll(f.Name())
	}()

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
		defer func() {
			f.Close()
			os.RemoveAll(f.Name())
		}()

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
		defer os.RemoveAll(wr)

		path := bazel.VersionPath(wr)
		actual, err := bazel.SafeVersionFromFile(path)
		g.Expect(err).ToNot(HaveOccurred())
		expected := &bazel.Version{}
		g.Expect(actual).To(Equal(expected))
	})
}

func TestWriteOutput(t *testing.T) {
	t.Run("Aspect version, Bazel version", func(t *testing.T) {
		g := NewWithT(t)
		v := &bazel.Version{
			Bazel:  "5.3.0",
			Aspect: "0.6.0",
		}
		actual := v.WriteOutput()
		expected := `aspect-build/0.6.0
5.3.0`
		g.Expect(actual).To(Equal(expected))
	})
	t.Run("no Aspect version, Bazel version", func(t *testing.T) {
		g := NewWithT(t)
		v := &bazel.Version{
			Bazel:  "5.3.0",
			Aspect: "",
		}
		actual := v.WriteOutput()
		expected := `5.3.0`
		g.Expect(actual).To(Equal(expected))
	})
	t.Run("Aspect version, no Bazel version", func(t *testing.T) {
		g := NewWithT(t)
		v := &bazel.Version{
			Bazel:  "",
			Aspect: "0.6.0",
		}
		actual := v.WriteOutput()
		expected := `aspect-build/0.6.0`
		g.Expect(actual).To(Equal(expected))
	})
	t.Run("no Aspect version, no Bazel version", func(t *testing.T) {
		g := NewWithT(t)
		v := &bazel.Version{
			Bazel:  "",
			Aspect: "",
		}
		actual := v.WriteOutput()
		expected := ``
		g.Expect(actual).To(Equal(expected))
	})
}

func TestWrite(t *testing.T) {
	g := NewWithT(t)
	v := &bazel.Version{
		Bazel:  "5.3.0",
		Aspect: "0.6.0",
	}
	var b bytes.Buffer
	err := v.Write(&b)
	g.Expect(err).ToNot(HaveOccurred())
	actual := b.String()
	expected := `aspect-build/0.6.0
5.3.0`
	g.Expect(actual).To(Equal(expected))
}

func TestWriteToFile(t *testing.T) {
	g := NewWithT(t)
	wr, err := os.MkdirTemp("", "wksp_root")
	g.Expect(err).ToNot(HaveOccurred())

	v := &bazel.Version{
		Bazel:  "5.3.0",
		Aspect: "0.6.0",
	}
	vp := bazel.VersionPath(wr)
	err = v.WriteToFile(vp)
	g.Expect(err).ToNot(HaveOccurred())

	data, err := os.ReadFile(vp)
	actual := string(data)
	expected := `aspect-build/0.6.0
5.3.0`
	g.Expect(actual).To(Equal(expected))
}

func TestInitAspect(t *testing.T) {
	g := NewWithT(t)
	v := &bazel.Version{}
	v.InitAspect()
	g.Expect(v.Aspect).To(Equal(buildinfo.PreStampRelease))
}
