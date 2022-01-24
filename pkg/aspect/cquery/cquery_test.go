/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package cquery_test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/cquery"
	"aspect.build/cli/pkg/aspect/query/shared"
	query_mock "aspect.build/cli/pkg/aspect/query/shared/mock"
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
			Spawn([]string{"cquery", "somepath(//cmd/aspect/query:query, @com_github_bazelbuild_bazelisk//core:go_default_library)"}).
			Return(0, nil)

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		q := cquery.New(streams, spawner, true)
		q.Presets = []*shared.PresetQuery{
			{
				Name:        "why",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?target, ?dependency)",
				Verb:        "cquery",
			},
		}

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
			Spawn([]string{"cquery", "somepath(//cmd/aspect/query:query, @com_github_bazelbuild_bazelisk//core:go_default_library)"}).
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

		q := &cquery.CQuery{
			Streams:       streams,
			Bzl:           spawner,
			IsInteractive: true,
			Prompt: func(label string) shared.PromptRunner {
				return promptRunner
			},
		}
		q.Presets = []*shared.PresetQuery{
			{
				Name:        "why",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?target, ?dependency)",
				Verb:        "cquery",
			},
		}
		err := q.Run(nil, []string{"why"})
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

		q := &cquery.CQuery{
			Streams:       streams,
			Bzl:           spawner,
			IsInteractive: true,
			Prompt: func(label string) shared.PromptRunner {
				return promptRunner
			},
		}
		q.Presets = []*shared.PresetQuery{
			{
				Name:        "why",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?target, ?dependency)",
				Verb:        "cquery",
			},
		}
		err := q.Run(cmd, []string{"why"})
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
			Spawn([]string{"cquery", "somepath(//cmd/aspect/query:query, @com_github_bazelbuild_bazelisk//core:go_default_library)"}).
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

		selectRunner := query_mock.NewMockSelectRunner(ctrl)
		selectRunner.
			EXPECT().
			Run().
			Return(1, "", nil).
			Times(1)

		q := &cquery.CQuery{
			Streams:       streams,
			Bzl:           spawner,
			IsInteractive: true,
			Prompt: func(label string) shared.PromptRunner {
				g.Expect(strings.Contains(label, "targettwo") || strings.Contains(label, "dependencytwo")).To(Equal(true))
				return promptRunner
			},
			Select: func(presetNames []string) shared.SelectRunner {
				return selectRunner
			},
		}
		q.Presets = []*shared.PresetQuery{
			{
				Name:        "why1",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?targetone, ?dependencyone)",
				Verb:        "cquery",
			},
			{
				Name:        "why2",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?targettwo, ?dependencytwo)",
				Verb:        "cquery",
			},
			{
				Name:        "why3",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?targetthree, ?dependencythree)",
				Verb:        "cquery",
			},
		}
		err := q.Run(nil, []string{})
		g.Expect(err).To(BeNil())
	})
}
