package test_test

import (
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"

	"aspect.build/cli/pkg/aspect/test"
	"aspect.build/cli/pkg/bazel/mock"
	"aspect.build/cli/pkg/ioutils"
)

// Embrace the stutter :)
func TestTest(t *testing.T) {

	t.Run("test calls bazel test", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		spawner := mock.NewMockSpawner(ctrl)
		spawner.
			EXPECT().
			Spawn([]string{"test"}).
			Return(0, nil)

		testCmd := test.New(ioutils.Streams{}, spawner)
		g.Expect(testCmd.Run(nil, []string{})).Should(Succeed())
	})
}
