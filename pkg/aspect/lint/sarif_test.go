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

package lint

import (
	"io"
	"strings"
	"testing"

	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	. "github.com/onsi/gomega"
)

func TestSarif(t *testing.T) {
	t.Run("processes clang tidy output -> sarif correctly", func(t *testing.T) {
		g := NewGomegaWithT(t)
		stdOutReader, stdOutWriter := io.Pipe()
		stdOut := new(strings.Builder)
		go func() {
			io.Copy(stdOut, stdOutReader)
		}()

		streams := ioutils.Streams{
			Stdout: stdOutWriter,
		}

		lintHandler := LintResultsFileHandler{Streams: streams}

		sarifJsonString, _ := lintHandler.toSarifJsonString("//speller/announce:announce", "AspectRulesLintClangTidy", clang_tidy_output)
		sarifJson, _ := lintHandler.toSarifJson(sarifJsonString)

		stdOutWriter.Close()
		stdOutReader.Close()

		g.Expect(len(sarifJson.Runs)).To(Equal(1))
		g.Expect(sarifJson.Runs[0].Tool.Driver.Name).To(Equal("ClangTidy"))
		g.Expect(len(sarifJson.Runs[0].Results)).To(Equal(2))
		g.Expect(sarifJson.Runs[0].Results[0].Message.Text).To(Equal("function is not thread safe [concurrency-mt-unsafe]"))
		g.Expect(sarifJson.Runs[0].Results[1].Message.Text).To(Equal("function is not thread safe [concurrency-mt-unsafe]"))
		g.Expect(sarifJson.Runs[0].Results[0].Locations[0].PhysicalLocation.ArtifactLocation.URI).To(Equal("speller/announce/announce.cc"))
		g.Expect(sarifJson.Runs[0].Results[1].Locations[0].PhysicalLocation.ArtifactLocation.URI).To(Equal("speller/announce/announce.cc"))
		g.Expect(sarifJson.Runs[0].Results[0].Locations[0].PhysicalLocation.Region.GetRdfRange().Start.Line).To(Equal(int32(19)))
		g.Expect(sarifJson.Runs[0].Results[1].Locations[0].PhysicalLocation.Region.GetRdfRange().Start.Line).To(Equal(int32(19)))
	})

	t.Run("determineRelativePath: returns relative paths untouched", func(t *testing.T) {
		g := NewGomegaWithT(t)

		// incomplete bazel label
		g.Expect(determineRelativePath("foo", "")).To(Equal("foo"))
		g.Expect(determineRelativePath("foo/bar/baz", "")).To(Equal("foo/bar/baz"))
		g.Expect(determineRelativePath("foo", "bar")).To(Equal("foo"))
		g.Expect(determineRelativePath("foo/bar/baz", "bar")).To(Equal("foo/bar/baz"))
		g.Expect(determineRelativePath("foo", "/bar")).To(Equal("foo"))
		g.Expect(determineRelativePath("foo/bar/baz", "/bar")).To(Equal("foo/bar/baz"))

		// normal bazel labels
		g.Expect(determineRelativePath("foo", "//foo")).To(Equal("foo"))
		g.Expect(determineRelativePath("foo/bar/baz", "//foo")).To(Equal("foo/bar/baz"))
		g.Expect(determineRelativePath("foo", "//foo:bar")).To(Equal("foo"))
		g.Expect(determineRelativePath("foo/bar/baz", "//foo:bar")).To(Equal("foo/bar/baz"))
		g.Expect(determineRelativePath("foo", "//foo/bar")).To(Equal("foo"))
		g.Expect(determineRelativePath("foo/bar/baz", "//foo/bar")).To(Equal("foo/bar/baz"))
		g.Expect(determineRelativePath("foo", "//foo/bar:baz")).To(Equal("foo"))
		g.Expect(determineRelativePath("foo/bar/baz", "//foo/bar:baz")).To(Equal("foo/bar/baz"))
	})

	t.Run("determineRelativePath: returns absolute paths as relative paths", func(t *testing.T) {
		g := NewGomegaWithT(t)

		// real examples
		g.Expect(determineRelativePath("/mnt/ephemeral/output/platform/__main__/sandbox/linux-sandbox/6210/execroot/com_vectara_platform/apiv2/common/src/main/java/com/vectara/apiv2/common/paging/PageKeys.java", "//apiv2/common/src/main/java/com/vectara/apiv2/common/paging:paging")).To(Equal("apiv2/common/src/main/java/com/vectara/apiv2/common/paging/PageKeys.java"))
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/769/execroot/_main/speller/lookup/lookup-test.cc", "//speller/lookup:lookup")).To(Equal("speller/lookup/lookup-test.cc"))
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/780/execroot/_main/speller/data_driven_tests/lookup-datatest.cc", "//speller/data_driven_tests:test-002")).To(Equal("speller/data_driven_tests/lookup-datatest.cc"))
		g.Expect(determineRelativePath("/private/var/tmp/_bazel_jesse/93d7e699c5e2019d94351d19b00be5a3/sandbox/darwin-sandbox/249/execroot/_main/speller/announce/announce.cc", "//speller/announce:announce")).To(Equal("speller/announce/announce.cc"))

		// execroot in label
		g.Expect(determineRelativePath("/some_path/sandbox/linux-sandbox/769/execroot/_main/execroot/foo/bar.baz", "//execroot:foo")).To(Equal("execroot/foo/bar.baz"))
		g.Expect(determineRelativePath("/some_path/sandbox/linux-sandbox/769/execroot/_main/execroot/foo/bar.baz", "//:foo")).To(Equal("execroot/foo/bar.baz"))
		g.Expect(determineRelativePath("/some_path/sandbox/linux-sandbox/769/execroot/_main/execroot/foo/bar.baz", "//execroot/foo:foo")).To(Equal("execroot/foo/bar.baz"))
		g.Expect(determineRelativePath("/some_path/sandbox/linux-sandbox/769/execroot/_main/foo/execroot/bar.baz", "//foo/execroot:execroot")).To(Equal("foo/execroot/bar.baz"))

		// Short labels
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/769/execroot/_main/speller/lookup/lookup-test.cc", "//speller/lookup")).To(Equal("speller/lookup/lookup-test.cc"))
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/780/execroot/_main/speller/data_driven_tests/lookup-datatest.cc", "//speller/data_driven_tests")).To(Equal("speller/data_driven_tests/lookup-datatest.cc"))
		g.Expect(determineRelativePath("/private/var/tmp/_bazel_jesse/93d7e699c5e2019d94351d19b00be5a3/sandbox/darwin-sandbox/249/execroot/_main/speller/announce/announce.cc", "//speller/announce")).To(Equal("speller/announce/announce.cc"))
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/769/execroot/_main/speller/lookup/lookup-test.cc", "//speller")).To(Equal("speller/lookup/lookup-test.cc"))
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/780/execroot/_main/speller/data_driven_tests/lookup-datatest.cc", "//speller")).To(Equal("speller/data_driven_tests/lookup-datatest.cc"))
		g.Expect(determineRelativePath("/private/var/tmp/_bazel_jesse/93d7e699c5e2019d94351d19b00be5a3/sandbox/darwin-sandbox/249/execroot/_main/speller/announce/announce.cc", "//speller")).To(Equal("speller/announce/announce.cc"))
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/769/execroot/_main/speller/lookup/lookup-test.cc", "//:lookup")).To(Equal("speller/lookup/lookup-test.cc"))
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/780/execroot/_main/speller/data_driven_tests/lookup-datatest.cc", "//:data_driven_tests")).To(Equal("speller/data_driven_tests/lookup-datatest.cc"))
		g.Expect(determineRelativePath("/private/var/tmp/_bazel_jesse/93d7e699c5e2019d94351d19b00be5a3/sandbox/darwin-sandbox/249/execroot/_main/speller/announce/announce.cc", "//:announce")).To(Equal("speller/announce/announce.cc"))
	})

	t.Run("determineRelativePath: returns absolute paths on regex or label error", func(t *testing.T) {
		g := NewGomegaWithT(t)

		// non bazel absolute path
		g.Expect(determineRelativePath("/some/path/foo/bar.baz", "//foo:foo")).To(Equal("/some/path/foo/bar.baz"))
		g.Expect(determineRelativePath("/some/path/foo/bar.baz", "//foo")).To(Equal("/some/path/foo/bar.baz"))

		// invalid labels
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/769/execroot/_main/speller/lookup/lookup-test.cc", "")).To(Equal("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/769/execroot/_main/speller/lookup/lookup-test.cc"))
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/780/execroot/_main/speller/data_driven_tests/lookup-datatest.cc", "")).To(Equal("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/780/execroot/_main/speller/data_driven_tests/lookup-datatest.cc"))
		g.Expect(determineRelativePath("/private/var/tmp/_bazel_jesse/93d7e699c5e2019d94351d19b00be5a3/sandbox/darwin-sandbox/249/execroot/_main/speller/announce/announce.cc", "")).To(Equal("/private/var/tmp/_bazel_jesse/93d7e699c5e2019d94351d19b00be5a3/sandbox/darwin-sandbox/249/execroot/_main/speller/announce/announce.cc"))
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/769/execroot/_main/speller/lookup/lookup-test.cc", "//foo")).To(Equal("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/769/execroot/_main/speller/lookup/lookup-test.cc"))
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/780/execroot/_main/speller/data_driven_tests/lookup-datatest.cc", "//foo")).To(Equal("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/780/execroot/_main/speller/data_driven_tests/lookup-datatest.cc"))
		g.Expect(determineRelativePath("/private/var/tmp/_bazel_jesse/93d7e699c5e2019d94351d19b00be5a3/sandbox/darwin-sandbox/249/execroot/_main/speller/announce/announce.cc", "//foo")).To(Equal("/private/var/tmp/_bazel_jesse/93d7e699c5e2019d94351d19b00be5a3/sandbox/darwin-sandbox/249/execroot/_main/speller/announce/announce.cc"))
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/769/execroot/_main/speller/lookup/lookup-test.cc", "speller/lookup")).To(Equal("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/769/execroot/_main/speller/lookup/lookup-test.cc"))
		g.Expect(determineRelativePath("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/780/execroot/_main/speller/data_driven_tests/lookup-datatest.cc", "speller/data_driven_tests")).To(Equal("/mnt/ephemeral/output/bazel-examples/__main__/sandbox/linux-sandbox/780/execroot/_main/speller/data_driven_tests/lookup-datatest.cc"))
		g.Expect(determineRelativePath("/private/var/tmp/_bazel_jesse/93d7e699c5e2019d94351d19b00be5a3/sandbox/darwin-sandbox/249/execroot/_main/speller/announce/announce.cc", "speller/announce")).To(Equal("/private/var/tmp/_bazel_jesse/93d7e699c5e2019d94351d19b00be5a3/sandbox/darwin-sandbox/249/execroot/_main/speller/announce/announce.cc"))
	})
}
