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

/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package bazel

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	. "github.com/onsi/gomega"

	"aspect.build/cli/pkg/ioutils"
)

var testTmpdir = os.Getenv("TEST_TMPDIR")
var workspaceDir = filepath.Join(testTmpdir, "project")
var workspaceFile = filepath.Join(workspaceDir, "WORKSPACE")
var workspaceOverrideDir = filepath.Join(testTmpdir, "project", "foo", "bar")
var wrapperOverridePath = filepath.Join(workspaceOverrideDir, wrapperPath)
var wrapperContents = []byte("#!/usr/bin/env bash\nprintf 'wrapper called'")

func init() {
	if err := os.Setenv("BAZELISK_HOME", testTmpdir); err != nil {
		panic(err)
	}
	if err := os.MkdirAll(workspaceDir, os.ModePerm); err != nil {
		panic(err)
	}
	if err := os.WriteFile(workspaceFile, []byte{}, 0644); err != nil {
		panic(err)
	}
	if err := os.MkdirAll(filepath.Dir(wrapperOverridePath), os.ModePerm); err != nil {
		panic(err)
	}
	if err := os.WriteFile(wrapperOverridePath, wrapperContents, 0777); err != nil {
		panic(err)
	}
}

func TestBazel(t *testing.T) {
	t.Run("when a custom environment is passed, it should be used by bazelisk", func(t *testing.T) {
		g := NewGomegaWithT(t)

		bzl := New(workspaceDir)

		env := []string{fmt.Sprintf("FOO=%s", "BAR")}
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		_, err := bzl.WithEnv(env).RunCommand(streams, "--print_env")
		g.Expect(err).To(Not(HaveOccurred()))
		g.Expect(stdout.String()).To(ContainSubstring("FOO=BAR"))
	})

	t.Run("when the workspace override directory is set, it should be used by bazelisk", func(t *testing.T) {
		g := NewGomegaWithT(t)

		bzl := New(workspaceOverrideDir)

		var out strings.Builder
		streams := ioutils.Streams{Stdout: &out, Stderr: &out}
		// workspaceOverrideDir is an unconventional location that has a tools/bazel to be used.
		// It must run the tools/bazel we placed under that location.
		_, err := bzl.RunCommand(streams, "build")
		g.Expect(err).To(Not(HaveOccurred()))
		g.Expect(out.String()).To(Equal("wrapper called"))
	})
}
