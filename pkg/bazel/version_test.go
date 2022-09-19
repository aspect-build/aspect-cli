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
	"testing"

	"aspect.build/cli/pkg/bazel"
	. "github.com/onsi/gomega"
)

func TestVersionPath(t *testing.T) {
	g := NewWithT(t)
	actual := bazel.VersionPath("/path/to/workspace")
	g.Expect(actual).To(Equal("/path/to/workspace/.bazelversion"))
}

func TestNewVersion(t *testing.T) {
	t.Error("IMPLEMENT ME!")
}

func TestNewVersionFromReader(t *testing.T) {
	t.Error("IMPLEMENT ME!")
}

func TestNewVersionFromFile(t *testing.T) {
	t.Error("IMPLEMENT ME!")
}

func TestSafeVersionFromFile(t *testing.T) {
	t.Run("when .bazelversion exists", func(t *testing.T) {
		t.Error("IMPLEMENT ME!")
	})
	t.Run("when .bazelversion does not exist", func(t *testing.T) {
		t.Error("IMPLEMENT ME!")
	})
}
