/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package bazel

import (
	"fmt"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"

	pathutils_mock "aspect.build/cli/pkg/pathutils/mock"
)

func TestBazel(t *testing.T) {
	t.Run("when getting the current working directory fails, an error is thrown", func(t *testing.T) {
		g := NewGomegaWithT(t)

		expectedErr := fmt.Errorf("failed to get working directory")

		osGetwd := func() (dir string, err error) {
			return "", expectedErr
		}

		bzl := &bazel{
			osGetwd:         osGetwd,
			workspaceFinder: nil,
		}

		_, err := bzl.Spawn([]string{"help"})
		g.Expect(err).To(MatchError(expectedErr))
	})

	t.Run("when the workspace finder fails, the interceptor fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		wd := "fake_working_directory/foo/bar"
		expectedErr := fmt.Errorf("failed to find yada yada yada")

		osGetwd := func() (dir string, err error) {
			return wd, nil
		}
		workspaceFinder := pathutils_mock.NewMockWorkspaceFinder(ctrl)
		workspaceFinder.EXPECT().
			Find(wd).
			Return("", expectedErr).
			Times(1)

		bzl := &bazel{
			osGetwd:         osGetwd,
			workspaceFinder: workspaceFinder,
		}

		_, err := bzl.Spawn([]string{"help"})
		g.Expect(err).To(MatchError(expectedErr))
	})

	t.Run("when the workspace finder returns empty, the interceptor fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		wd := "fake_working_directory/foo/bar"
		expectedErrStr := fmt.Sprintf("failed to find bazel workspace root: the current working directory %q is not a Bazel workspace", wd)

		osGetwd := func() (dir string, err error) {
			return wd, nil
		}
		workspaceFinder := pathutils_mock.NewMockWorkspaceFinder(ctrl)
		workspaceFinder.EXPECT().
			Find(wd).
			Return("", nil).
			Times(1)

		bzl := &bazel{
			osGetwd:         osGetwd,
			workspaceFinder: workspaceFinder,
		}

		_, err := bzl.Spawn([]string{"help"})
		g.Expect(err).To(MatchError(expectedErrStr))
	})
}
