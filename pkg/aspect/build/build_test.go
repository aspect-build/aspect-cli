/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package build_test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"

	"aspect.build/cli/pkg/aspect/build"
	"aspect.build/cli/pkg/aspecterrors"
	bazel_mock "aspect.build/cli/pkg/bazel/mock"
	"aspect.build/cli/pkg/ioutils"
	bep_mock "aspect.build/cli/pkg/plugin/system/bep/mock"
)

func TestBuild(t *testing.T) {
	t.Run("when the bazel runner fails, the aspect build fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		streams := ioutils.Streams{}
		bzl := bazel_mock.NewMockBazel(ctrl)
		expectErr := &aspecterrors.ExitError{
			Err:      fmt.Errorf("failed to run bazel build"),
			ExitCode: 5,
		}
		bzl.
			EXPECT().
			Spawn([]string{"build", "--bes_backend=grpc://127.0.0.1:12345", "//..."}).
			Return(expectErr.ExitCode, expectErr.Err)
		besBackend := bep_mock.NewMockBESBackend(ctrl)
		besBackend.
			EXPECT().
			Addr().
			Return("127.0.0.1:12345").
			Times(1)
		besBackend.
			EXPECT().
			Errors().
			Times(1)

		b := build.New(streams, bzl)
		err := b.Run([]string{"//..."}, besBackend)

		g.Expect(err).To(MatchError(expectErr))
	})

	t.Run("when the BES backend contains errors, the aspect build fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stderr strings.Builder
		streams := ioutils.Streams{Stderr: &stderr}
		bzl := bazel_mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			Spawn([]string{"build", "--bes_backend=grpc://127.0.0.1:12345", "//..."}).
			Return(0, nil)
		besBackend := bep_mock.NewMockBESBackend(ctrl)
		besBackend.
			EXPECT().
			Addr().
			Return("127.0.0.1:12345").
			Times(1)
		besBackend.
			EXPECT().
			Errors().
			Return([]error{
				fmt.Errorf("error 1"),
				fmt.Errorf("error 2"),
			}).
			Times(1)

		b := build.New(streams, bzl)
		err := b.Run([]string{"//..."}, besBackend)

		g.Expect(err).To(MatchError(&aspecterrors.ExitError{ExitCode: 1}))
		g.Expect(stderr.String()).To(Equal("Error: failed to run build command: error 1\nError: failed to run build command: error 2\n"))
	})

	t.Run("when the bazel runner succeeds, the aspect build succeeds", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		streams := ioutils.Streams{}
		bzl := bazel_mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			Spawn([]string{"build", "--bes_backend=grpc://127.0.0.1:12345", "//..."}).
			Return(0, nil)
		besBackend := bep_mock.NewMockBESBackend(ctrl)
		besBackend.
			EXPECT().
			Addr().
			Return("127.0.0.1:12345").
			Times(1)
		besBackend.
			EXPECT().
			Errors().
			Times(1)

		b := build.New(streams, bzl)
		err := b.Run([]string{"//..."}, besBackend)

		g.Expect(err).To(BeNil())
	})
}
