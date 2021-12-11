/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package interceptors

import (
	"context"
	"fmt"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
	"github.com/spf13/cobra"

	pathutils_mock "aspect.build/cli/pkg/pathutils/mock"
)

func TestWorkspaceRootInterceptor(t *testing.T) {
	t.Run("when getting the current working directory fails, the interceptor fails", func(t *testing.T) {
		g := NewGomegaWithT(t)

		expectedErr := fmt.Errorf("failed to get working directory")

		osGetwd := func() (dir string, err error) {
			return "", expectedErr
		}

		ctx := context.Background()
		cmd := &cobra.Command{Use: "fake"}

		err := workspaceRootInterceptor(osGetwd, nil)(ctx, cmd, nil, nil)
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

		ctx := context.Background()
		cmd := &cobra.Command{Use: "fake"}

		err := workspaceRootInterceptor(osGetwd, workspaceFinder)(ctx, cmd, nil, nil)
		g.Expect(err).To(MatchError(expectedErr))
	})

	t.Run("when the workspace finder returns empty, the interceptor fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		wd := "fake_working_directory/foo/bar"
		cmdName := "fake"
		expectedErrStr := fmt.Sprintf("failed to run command %q: the current working directory %q is not a Bazel workspace", cmdName, wd)

		osGetwd := func() (dir string, err error) {
			return wd, nil
		}
		workspaceFinder := pathutils_mock.NewMockWorkspaceFinder(ctrl)
		workspaceFinder.EXPECT().
			Find(wd).
			Return("", nil).
			Times(1)

		ctx := context.Background()
		cmd := &cobra.Command{Use: cmdName}

		err := workspaceRootInterceptor(osGetwd, workspaceFinder)(ctx, cmd, nil, nil)
		g.Expect(err).To(MatchError(expectedErrStr))
	})

	t.Run("succeeds", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		wd := "fake_working_directory/foo/bar"
		workspacePath := "fake_working_directory/WORKSPACE"
		expectedWorkspaceRoot := "fake_working_directory"

		osGetwd := func() (dir string, err error) {
			return wd, nil
		}
		workspaceFinder := pathutils_mock.NewMockWorkspaceFinder(ctrl)
		workspaceFinder.EXPECT().
			Find(wd).
			Return(workspacePath, nil).
			Times(1)

		ctx := context.Background()
		cmd := &cobra.Command{Use: "fake"}
		args := []string{"foo", "bar"}
		next := func(_ctx context.Context, _cmd *cobra.Command, _args []string) error {
			ctx := context.WithValue(ctx, WorkspaceRootKey, expectedWorkspaceRoot)
			g.Expect(_ctx).To(Equal(ctx))
			g.Expect(_cmd).To(Equal(cmd))
			g.Expect(_args).To(Equal(args))
			return nil
		}

		err := workspaceRootInterceptor(osGetwd, workspaceFinder)(ctx, cmd, args, next)
		g.Expect(err).To(BeNil())
	})
}
