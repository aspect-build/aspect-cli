/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package clean_test

import (
	"fmt"
	"io/ioutil"
	"os"
	"strings"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
	"github.com/spf13/viper"

	"aspect.build/cli/pkg/aspect/clean"
	"aspect.build/cli/pkg/bazel/mock"
	"aspect.build/cli/pkg/ioutils"
)

type confirm struct{}

func (p confirm) Run() (string, error) {
	return "", nil
}

type deny struct{}

func (p deny) Run() (string, error) {
	return "", fmt.Errorf("said no")
}

type chooseReclaim struct{}

func (p chooseReclaim) Run() (int, string, error) {
	return 0, clean.ReclaimOption, nil
}

type chooseNonIncremental struct{}

func (p chooseNonIncremental) Run() (int, string, error) {
	return 2, clean.NonIncrementalOption, nil
}

type chooseInvalidateRepos struct{}

func (p chooseInvalidateRepos) Run() (int, string, error) {
	return 3, clean.InvalidateReposOption, nil
}

type chooseWorkaround struct{}

func (p chooseWorkaround) Run() (int, string, error) {
	return 4, clean.WorkaroundOption, nil
}

func TestClean(t *testing.T) {

	t.Run("clean calls bazel clean", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		spawner := mock.NewMockSpawner(ctrl)
		spawner.
			EXPECT().
			Spawn([]string{"clean"}).
			Return(0, nil)

		cleanCmd := clean.New(ioutils.Streams{}, spawner)
		g.Expect(cleanCmd.Run(false)).Should(Succeed())
	})

	t.Run("clean expunge calls bazel clean expunge", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		spawner := mock.NewMockSpawner(ctrl)
		spawner.
			EXPECT().
			Spawn([]string{"clean", "--expunge"}).
			Return(0, nil)

		cleanCmd := clean.New(ioutils.Streams{}, spawner)
		cleanCmd.Expunge = true
		g.Expect(cleanCmd.Run(false)).Should(Succeed())
	})

	t.Run("clean expunge_async calls bazel clean expunge_async", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		spawner := mock.NewMockSpawner(ctrl)
		spawner.
			EXPECT().
			Spawn([]string{"clean", "--expunge_async"}).
			Return(0, nil)

		cleanCmd := clean.New(ioutils.Streams{}, spawner)
		cleanCmd.ExpungeAsync = true
		g.Expect(cleanCmd.Run(false)).Should(Succeed())
	})

	t.Run("interactive clean prompts for usage, option 1", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		spawner := mock.NewMockSpawner(ctrl)
		spawner.
			EXPECT().
			Spawn([]string{"clean"}).
			Return(0, nil)

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		cleanCmd := clean.New(streams, spawner)

		cleanCmd.Behavior = chooseReclaim{}
		cleanCmd.Remember = deny{}

		g.Expect(cleanCmd.Run(true)).Should(Succeed())
		g.Expect(stdout.String()).To(ContainSubstring("skip this prompt"))
	})

	t.Run("interactive clean prompts for usage, option 1 and save", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		spawner := mock.NewMockSpawner(ctrl)
		spawner.
			EXPECT().
			Spawn([]string{"clean"}).
			Return(0, nil).AnyTimes()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		cleanCmd1 := clean.New(streams, spawner)

		viper := *viper.New()
		cfg, err := os.CreateTemp(os.Getenv("TEST_TMPDIR"), "cfg***.ini")
		g.Expect(err).To(BeNil())

		viper.SetConfigFile(cfg.Name())
		cleanCmd1.Behavior = chooseReclaim{}
		cleanCmd1.Remember = confirm{}
		cleanCmd1.Prefs = viper
		g.Expect(cleanCmd1.Run(true)).Should(Succeed())
		g.Expect(stdout.String()).To(ContainSubstring("skip this prompt"))

		// Recorded your preference for next time
		content, err := ioutil.ReadFile(cfg.Name())
		g.Expect(err).To(BeNil())
		g.Expect(string(content)).To(Equal("[clean]\nskip_prompt=true\n\n"))

		// If we run it again, there should be no prompt
		cleanCmd2 := clean.New(streams, spawner)
		cleanCmd2.Prefs = viper
		g.Expect(cleanCmd2.Run(true)).Should(Succeed())
	})

	t.Run("interactive clean prompts for usage, option 2", func(t *testing.T) {
		g := NewGomegaWithT(t)
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}

		cleanCmd := clean.New(streams, nil)
		cleanCmd.Behavior = chooseNonIncremental{}
		g.Expect(cleanCmd.Run(true)).Should(Succeed())
		g.Expect(stdout.String()).To(ContainSubstring("use the --output_base flag"))
	})

	t.Run("interactive clean prompts for usage, option 3", func(t *testing.T) {
		g := NewGomegaWithT(t)
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}

		cleanCmd := clean.New(streams, nil)
		cleanCmd.Behavior = chooseInvalidateRepos{}
		g.Expect(cleanCmd.Run(true)).Should(Succeed())
		g.Expect(stdout.String()).To(ContainSubstring("aspect sync --configure"))
	})

	t.Run("interactive clean prompts for usage, option 5", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		spawner := mock.NewMockSpawner(ctrl)
		spawner.
			EXPECT().
			Spawn([]string{"clean"}).
			Return(0, nil)

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}

		cleanCmd := clean.New(streams, spawner)
		cleanCmd.Behavior = chooseWorkaround{}
		cleanCmd.Workaround = confirm{}
		g.Expect(cleanCmd.Run(true)).Should(Succeed())
		g.Expect(stdout.String()).To(ContainSubstring("recommend you file a bug"))
	})
}
