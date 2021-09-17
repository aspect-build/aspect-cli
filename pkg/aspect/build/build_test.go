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
	bazel_mock "aspect.build/cli/pkg/bazel_mock"
	"aspect.build/cli/pkg/ioutils"
)

func TestBuild(t *testing.T) {
	t.Run("when the bazel runner fails, the aspect build fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		spawner := bazel_mock.NewMockSpawner(ctrl)
		expectErr := fmt.Errorf("failed to run bazel build")
		spawner.
			EXPECT().
			Spawn(gomock.Any()).
			Return(1, expectErr)

		b := build.New(streams, spawner)
		err := b.Run(nil, nil)

		g.Expect(err).To(Equal(expectErr))
	})

	t.Run("when the bazel runner succeeds, the aspect build succeeds", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		spawner := bazel_mock.NewMockSpawner(ctrl)
		spawner.
			EXPECT().
			Spawn(gomock.Any()).
			Return(0, nil)

		b := build.New(streams, spawner)
		err := b.Run(nil, nil)

		g.Expect(err).To(BeNil())
	})
}
