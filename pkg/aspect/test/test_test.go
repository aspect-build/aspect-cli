package test_test

import (
	"testing"

	"aspect.build/cli/pkg/aspect/test"
	"aspect.build/cli/pkg/bazel/mock"
	"aspect.build/cli/pkg/ioutils"
	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
)

// Embrace the stutter :)
func TestTest(t *testing.T) {
	t.Run("test calls bazel test", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		bzl := mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			Spawn([]string{"test"}).
			Return(0, nil)

		b := test.New(ioutils.Streams{}, bzl)
		g.Expect(b.Run(nil, []string{})).Should(Succeed())
	})
}
