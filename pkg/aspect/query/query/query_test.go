/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package query_test

import (
	"fmt"
	"os"
	"strings"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	shared "aspect.build/cli/pkg/aspect/query"
	query_mock "aspect.build/cli/pkg/aspect/query/mock"
	"aspect.build/cli/pkg/aspect/query/query"
	bazel_mock "aspect.build/cli/pkg/bazel/mock"
	"aspect.build/cli/pkg/ioutils"
)

func TestQuery(t *testing.T) {
	t.Run("long version of preset query calls directly down to bazel query", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		spawner := bazel_mock.NewMockBazel(ctrl)
		spawner.
			EXPECT().
			Spawn([]string{"query", "somepath(//cmd/aspect/query:query, @com_github_bazelbuild_bazelisk//core:go_default_library)"}).
			Return(0, nil)

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		q := query.New(streams, spawner, true)
		q.Presets = []*shared.PresetQuery{
			{
				Name:        "why",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?target, ?dependency)",
				Verb:        "query",
			},
		}

		viper := *viper.New()
		cfg, err := os.CreateTemp(os.Getenv("TEST_TMPDIR"), "cfg***.ini")
		g.Expect(err).To(BeNil())

		viper.SetConfigFile(cfg.Name())
		q.Prefs = viper

		g.Expect(q.Run(nil, []string{"why", "//cmd/aspect/query:query", "@com_github_bazelbuild_bazelisk//core:go_default_library"})).Should(Succeed())
	})

	t.Run("query can be selected by default and will prompt for inputs", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}

		spawner := bazel_mock.NewMockBazel(ctrl)
		spawner.
			EXPECT().
			Spawn([]string{"query", "somepath(//cmd/aspect/query:query, @com_github_bazelbuild_bazelisk//core:go_default_library)"}).
			Return(0, nil)

		promptRunner := query_mock.NewMockPromptRunner(ctrl)
		gomock.InOrder(
			promptRunner.
				EXPECT().
				Run().
				Return("//cmd/aspect/query:query", nil).
				Times(1),
			promptRunner.
				EXPECT().
				Run().
				Return("@com_github_bazelbuild_bazelisk//core:go_default_library", nil).
				Times(1),
		)

		confirmationRunner := query_mock.NewMockConfirmationRunner(ctrl)
		gomock.InOrder(
			confirmationRunner.
				EXPECT().
				Run().
				Return("Y", nil).
				Times(1),
			confirmationRunner.
				EXPECT().
				Run().
				Return("Y", nil).
				Times(1),
		)

		q := &query.Query{
			Streams:       streams,
			Bzl:           spawner,
			IsInteractive: true,
			Prompt: func(label string) shared.PromptRunner {
				return promptRunner
			},
			Confirmation: func(question string) shared.ConfirmationRunner {
				return confirmationRunner
			},
		}
		q.Presets = []*shared.PresetQuery{
			{
				Name:        "why",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?target, ?dependency)",
				Verb:        "query",
			},
		}

		viper := *viper.New()
		cfg, err := os.CreateTemp(os.Getenv("TEST_TMPDIR"), "cfg***.ini")
		g.Expect(err).To(BeNil())

		viper.SetConfigFile(cfg.Name())
		q.Prefs = viper
		err = q.Run(nil, []string{"why"})
		g.Expect(err).To(BeNil())
	})

	t.Run("a thrown error while prompting for input is handled", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		expectedError := fmt.Errorf("The prompt failed!")

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}

		spawner := bazel_mock.NewMockBazel(ctrl)

		cmd := &cobra.Command{Use: "fake"}

		promptRunner := query_mock.NewMockPromptRunner(ctrl)
		gomock.InOrder(
			promptRunner.
				EXPECT().
				Run().
				Return("//foo", nil).
				Times(1),
			promptRunner.
				EXPECT().
				Run().
				Return("", expectedError).
				Times(1),
		)

		confirmationRunner := query_mock.NewMockConfirmationRunner(ctrl)
		gomock.InOrder(
			confirmationRunner.
				EXPECT().
				Run().
				Return("Y", nil).
				Times(1),
			confirmationRunner.
				EXPECT().
				Run().
				Return("Y", nil).
				Times(1),
		)

		q := &query.Query{
			Streams:       streams,
			Bzl:           spawner,
			IsInteractive: true,
			Prompt: func(label string) shared.PromptRunner {
				return promptRunner
			},
			Confirmation: func(question string) shared.ConfirmationRunner {
				return confirmationRunner
			},
		}
		q.Presets = []*shared.PresetQuery{
			{
				Name:        "why",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?target, ?dependency)",
				Verb:        "query",
			},
		}

		viper := *viper.New()
		cfg, err := os.CreateTemp(os.Getenv("TEST_TMPDIR"), "cfg***.ini")
		g.Expect(err).To(BeNil())

		viper.SetConfigFile(cfg.Name())
		q.Prefs = viper

		err = q.Run(cmd, []string{"why"})
		g.Expect(err).To(MatchError(expectedError))
	})

	t.Run("will prompt user to select a preset query", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}

		spawner := bazel_mock.NewMockBazel(ctrl)
		spawner.
			EXPECT().
			Spawn([]string{"query", "somepath(//cmd/aspect/query:query, @com_github_bazelbuild_bazelisk//core:go_default_library)"}).
			Return(0, nil)

		promptRunner := query_mock.NewMockPromptRunner(ctrl)
		gomock.InOrder(
			promptRunner.
				EXPECT().
				Run().
				Return("//cmd/aspect/query:query", nil).
				Times(1),
			promptRunner.
				EXPECT().
				Run().
				Return("@com_github_bazelbuild_bazelisk//core:go_default_library", nil).
				Times(1),
		)

		confirmationRunner := query_mock.NewMockConfirmationRunner(ctrl)
		gomock.InOrder(
			confirmationRunner.
				EXPECT().
				Run().
				Return("N", fmt.Errorf("")).
				Times(1),
			confirmationRunner.
				EXPECT().
				Run().
				Return("N", fmt.Errorf("")).
				Times(1),
		)

		selectRunner := query_mock.NewMockSelectRunner(ctrl)
		selectRunner.
			EXPECT().
			Run().
			Return(1, "", nil).
			Times(1)

		q := &query.Query{
			Streams:       streams,
			Bzl:           spawner,
			IsInteractive: true,
			Prompt: func(label string) shared.PromptRunner {
				fmt.Println("label:", label)
				g.Expect(strings.Contains(label, "targettwo") || strings.Contains(label, "dependencytwo")).To(Equal(true))
				return promptRunner
			},
			Select: func(presetNames []string) shared.SelectRunner {
				return selectRunner
			},
			Confirmation: func(question string) shared.ConfirmationRunner {
				return confirmationRunner
			},
		}
		q.Presets = []*shared.PresetQuery{
			{
				Name:        "why1",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?targetone, ?dependencyone)",
				Verb:        "query",
			},
			{
				Name:        "why2",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?targettwo, ?dependencytwo)",
				Verb:        "query",
			},
			{
				Name:        "why3",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?targetthree, ?dependencythree)",
				Verb:        "query",
			},
		}

		viper := *viper.New()
		cfg, err := os.CreateTemp(os.Getenv("TEST_TMPDIR"), "cfg***.ini")
		g.Expect(err).To(BeNil())

		viper.SetConfigFile(cfg.Name())
		q.Prefs = viper
		err = q.Run(nil, []string{})
		g.Expect(err).To(BeNil())
	})
}
