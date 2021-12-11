/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package pathutils

import (
	"fmt"
	"io/fs"
	"os"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
	"github.com/spf13/cobra"

	pathutils_mock "aspect.build/cli/pkg/pathutils/mock"
	stdlib_mock "aspect.build/cli/pkg/stdlib/mock"
)

func TestInvokeCmdInsideWorkspace(t *testing.T) {
	t.Run("when the workspace finder fails, the returned function fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		wd := "fake_working_directory/foo/bar"
		expectedErr := fmt.Errorf("failed to find yada yada yada")

		finder := pathutils_mock.NewMockFinder(ctrl)
		finder.EXPECT().
			Find(wd).
			Return("", expectedErr).
			Times(1)

		cmd := &cobra.Command{Use: "fake"}

		err := invokeCmdInsideWorkspace(finder, wd, nil)(cmd, nil)
		g.Expect(err).To(MatchError(expectedErr))
	})

	t.Run("when the workspace finder returns empty, the returned function fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		wd := "fake_working_directory/foo/bar"
		cmdName := "fake"
		expectedErrStr := fmt.Sprintf("failed to run command %q: the current working directory %q is not a Bazel workspace", cmdName, wd)

		finder := pathutils_mock.NewMockFinder(ctrl)
		finder.EXPECT().
			Find(wd).
			Return("", nil).
			Times(1)

		cmd := &cobra.Command{Use: cmdName}

		err := invokeCmdInsideWorkspace(finder, wd, nil)(cmd, nil)
		g.Expect(err).To(MatchError(expectedErrStr))
	})

	t.Run("succeeds", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		wd := "fake_working_directory/foo/bar"
		workspacePath := "fake_working_directory/WORKSPACE"
		expectedWorkspaceRoot := "fake_working_directory"

		finder := pathutils_mock.NewMockFinder(ctrl)
		finder.EXPECT().
			Find(wd).
			Return(workspacePath, nil).
			Times(1)

		cmd := &cobra.Command{Use: "fake"}
		args := []string{"foo", "bar"}

		err := invokeCmdInsideWorkspace(finder, wd, func(workspaceRoot string, _cmd *cobra.Command, _args []string) (exitErr error) {
			g.Expect(workspaceRoot).To(Equal(expectedWorkspaceRoot))
			g.Expect(_cmd).To(Equal(cmd))
			g.Expect(_args).To(Equal(args))
			return nil
		})(cmd, args)
		g.Expect(err).To(BeNil())
	})
}

func TestWorkspaceFinder(t *testing.T) {
	t.Run("when os.Stat fails, Find fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		wd := "fake_working_directory/foo/bar"
		expectedErr := fmt.Errorf("os.Stat failed")

		finder := &workspaceFinder{osStat: func(s string) (fs.FileInfo, error) {
			return nil, expectedErr
		}}
		workspacePath, err := finder.Find(wd)
		g.Expect(workspacePath).To(BeEmpty())
		g.Expect(err).To(MatchError(expectedErr))
	})

	t.Run("when a WORKSPACE is not found, the returned workspacePath is empty", func(t *testing.T) {
		g := NewGomegaWithT(t)

		// We also make sure that Find doesn't get into an infinite loop.
		wds := []string{
			"/level_1/level_2",
			"/level_1/level_2/level_3",
			"level_1/level_2",
			"level_1",
			"level_1/",
			"/level_1/",
			"/level_1/level_2/",
			".",
			"/",
			"",
		}

		for _, wd := range wds {
			finder := &workspaceFinder{osStat: func(s string) (fs.FileInfo, error) {
				return nil, os.ErrNotExist
			}}
			workspacePath, err := finder.Find(wd)
			g.Expect(workspacePath).To(BeEmpty())
			g.Expect(err).To(BeNil())
		}
	})

	t.Run("succeeds", func(t *testing.T) {
		t.Run("case 1", func(t *testing.T) {
			g := NewGomegaWithT(t)
			ctrl := gomock.NewController(t)
			defer ctrl.Finish()

			wd := "fake_working_directory/foo/bar"

			fsFileInfo := stdlib_mock.NewMockFSFileInfo(ctrl)
			fsFileInfo.EXPECT().
				IsDir().
				Return(false).
				Times(1)

			finder := &workspaceFinder{osStat: func(s string) (fs.FileInfo, error) {
				return fsFileInfo, nil
			}}
			workspacePath, err := finder.Find(wd)
			g.Expect(workspacePath).To(Equal("fake_working_directory/foo/bar/WORKSPACE"))
			g.Expect(err).To(BeNil())
		})
		t.Run("case 2", func(t *testing.T) {
			g := NewGomegaWithT(t)
			ctrl := gomock.NewController(t)
			defer ctrl.Finish()

			wd := "fake_working_directory/foo/bar"

			fsFileInfo := stdlib_mock.NewMockFSFileInfo(ctrl)
			gomock.InOrder(
				fsFileInfo.EXPECT().
					IsDir().
					Return(true).
					Times(1),
				fsFileInfo.EXPECT().
					IsDir().
					Return(false).
					Times(1),
			)

			finder := &workspaceFinder{osStat: func(s string) (fs.FileInfo, error) {
				return fsFileInfo, nil
			}}
			workspacePath, err := finder.Find(wd)
			g.Expect(workspacePath).To(Equal("fake_working_directory/foo/bar/WORKSPACE.bazel"))
			g.Expect(err).To(BeNil())
		})
		t.Run("case 3", func(t *testing.T) {
			g := NewGomegaWithT(t)
			ctrl := gomock.NewController(t)
			defer ctrl.Finish()

			wd := "fake_working_directory/foo/bar"

			fsFileInfo := stdlib_mock.NewMockFSFileInfo(ctrl)
			gomock.InOrder(
				fsFileInfo.EXPECT().
					IsDir().
					Return(true).
					Times(1),
				fsFileInfo.EXPECT().
					IsDir().
					Return(true).
					Times(1),
				fsFileInfo.EXPECT().
					IsDir().
					Return(false).
					Times(1),
			)

			finder := &workspaceFinder{osStat: func(s string) (fs.FileInfo, error) {
				return fsFileInfo, nil
			}}
			workspacePath, err := finder.Find(wd)
			g.Expect(workspacePath).To(Equal("fake_working_directory/foo/WORKSPACE"))
			g.Expect(err).To(BeNil())
		})
		t.Run("case 4", func(t *testing.T) {
			g := NewGomegaWithT(t)
			ctrl := gomock.NewController(t)
			defer ctrl.Finish()

			wd := "fake_working_directory/foo/bar"

			fsFileInfo := stdlib_mock.NewMockFSFileInfo(ctrl)
			gomock.InOrder(
				fsFileInfo.EXPECT().
					IsDir().
					Return(true).
					Times(1),
				fsFileInfo.EXPECT().
					IsDir().
					Return(true).
					Times(1),
				fsFileInfo.EXPECT().
					IsDir().
					Return(true).
					Times(1),
				fsFileInfo.EXPECT().
					IsDir().
					Return(false).
					Times(1),
			)

			finder := &workspaceFinder{osStat: func(s string) (fs.FileInfo, error) {
				return fsFileInfo, nil
			}}
			workspacePath, err := finder.Find(wd)
			g.Expect(workspacePath).To(Equal("fake_working_directory/foo/WORKSPACE.bazel"))
			g.Expect(err).To(BeNil())
		})
	})
}
