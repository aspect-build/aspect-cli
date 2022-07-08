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

	workspace_mock "aspect.build/cli/pkg/bazel/workspace/mock"
)

func TestBazel(t *testing.T) {
	t.Run("when the workspace finder fails, the interceptor fails", func(t *testing.T) {
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

		_, err := bzl.Spawn([]string{"help"})
		g.Expect(err).To(MatchError(expectedErr))
	})
}
