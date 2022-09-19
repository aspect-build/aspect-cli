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
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"aspect.build/cli/pkg/bazel/workspace"
	workspace_mock "aspect.build/cli/pkg/bazel/workspace/mock"
	"aspect.build/cli/pkg/ioutils"
	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
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
	if err := os.Chdir(workspaceDir); err != nil {
		panic(err)
	}
}

func TestBazel(t *testing.T) {
	t.Run("when the workspace finder fails, Spawn fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		expectedErr := fmt.Errorf("failed to find yada yada yada")

		workspaceFinder := workspace_mock.NewMockFinder(ctrl)
		workspaceFinder.EXPECT().
			Find().
			Return("", expectedErr).
			Times(1)

		bzl := &bazel{
			workspaceFinder: workspaceFinder,
		}

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		_, err := bzl.Spawn([]string{"--print_env"}, streams)
		g.Expect(err).To(MatchError(expectedErr))
	})

	t.Run("when a custom environment is passed, it should be used by bazelisk", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		workspaceFinder := workspace_mock.NewMockFinder(ctrl)
		workspaceFinder.EXPECT().
			Find().
			Return("", nil).
			Times(1)

		bzl := &bazel{
			workspaceFinder: workspaceFinder,
		}

		env := []string{fmt.Sprintf("FOO=%s", "BAR")}
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		_, err := bzl.WithEnv(env).Spawn([]string{"--print_env"}, streams)
		g.Expect(err).To(Not(HaveOccurred()))
		g.Expect(stdout.String()).To(ContainSubstring("FOO=BAR"))
	})

	t.Run("when the workspace override directory is set, it should be used by bazelisk", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		workspaceFinder := workspace_mock.NewMockFinder(ctrl)
		workspaceFinder.EXPECT().
			Find().
			Times(0)

		bzl := &bazel{
			workspaceFinder: workspaceFinder,
		}

		var out strings.Builder
		streams := ioutils.Streams{Stdout: &out, Stderr: &out}
		// workspaceOverrideDir is an unconventional location that has a tools/bazel to be used.
		// It must run the tools/bazel we placed under that location.
		_, err := bzl.WithOverrideWorkspaceRoot(workspaceOverrideDir).Spawn([]string{"build"}, streams)
		g.Expect(err).To(Not(HaveOccurred()))
		g.Expect(out.String()).To(Equal("wrapper called"))
	})
}

func TestWorkspaceRoot(t *testing.T) {
	// Skipping the override
	t.Run("finds the workspace root", func(t *testing.T) {
		g := NewWithT(t)

		curWd, err := os.Getwd()
		g.Expect(err).ToNot(HaveOccurred())

		wr := filepath.Join(curWd, "wksp_root")
		err = os.Mkdir(wr, 0750)
		g.Expect(err).ToNot(HaveOccurred())
		defer os.RemoveAll(wr)

		workspace, err := os.Create(filepath.Join(wr, "WORKSPACE"))
		_, err = io.WriteString(workspace, `workspace(name = "temp_workspace")`)
		workspace.Close()

		defer os.Chdir(curWd)
		os.Chdir(wr)

		b := New()
		actual, err := b.WorkspaceRoot()
		g.Expect(err).ToNot(HaveOccurred())
		g.Expect(actual).To(Equal(wr))
	})
	t.Run("with override", func(t *testing.T) {
		g := NewWithT(t)

		curWd, err := os.Getwd()
		g.Expect(err).ToNot(HaveOccurred())

		wr := filepath.Join(curWd, "wksp_root")
		err = os.Mkdir(wr, 0750)
		g.Expect(err).ToNot(HaveOccurred())
		defer os.RemoveAll(wr)

		wksp, err := os.Create(filepath.Join(wr, "WORKSPACE"))
		_, err = io.WriteString(wksp, `workspace(name = "temp_workspace")`)
		wksp.Close()

		b := &bazel{
			workspaceFinder: workspace.DefaultFinder,
		}
		bzl := b.WithOverrideWorkspaceRoot(wr)
		actual, err := bzl.WorkspaceRoot()
		g.Expect(err).ToNot(HaveOccurred())
		g.Expect(actual).To(Equal(wr))
	})
}
