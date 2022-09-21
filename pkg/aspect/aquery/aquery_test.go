/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package aquery_test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/aquery"
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

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		spawner := bazel_mock.NewMockBazel(ctrl)
		spawner.
			EXPECT().
			RunCommand([]string{"aquery", "somepath(//cmd/aspect/query:query, @com_github_bazelbuild_bazelisk//core:go_default_library)"}, streams).
			Return(0, nil)

		q := aquery.New(streams, spawner, true)
		q.Presets = []*shared.PresetQuery{
			{
				Name:        "why",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?target, ?dependency)",
				Verb:        "aquery",
			},
		}

		cmd := &cobra.Command{Use: "aquery"}
		g.Expect(q.Run(cmd, []string{"why", "//cmd/aspect/query:query", "@com_github_bazelbuild_bazelisk//core:go_default_library"})).Should(Succeed())
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
			RunCommand([]string{"aquery", "somepath(//cmd/aspect/query:query, @com_github_bazelbuild_bazelisk//core:go_default_library)"}, streams).
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

		q := &aquery.AQuery{
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
				Verb:        "aquery",
			},
		}
		cmd := &cobra.Command{Use: "aquery"}
		err := q.Run(cmd, []string{"why"})
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

		q := &aquery.AQuery{
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
				Verb:        "aquery",
			},
		}
		cmd := &cobra.Command{Use: "aquery"}
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
			RunCommand([]string{"aquery", "somepath(//cmd/aspect/query:query, @com_github_bazelbuild_bazelisk//core:go_default_library)"}, streams).
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

		q := &aquery.AQuery{
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
				Verb:        "aquery",
			},
			{
				Name:        "why2",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?targettwo, ?dependencytwo)",
				Verb:        "aquery",
			},
			{
				Name:        "why3",
				Description: "Determine why a target depends on another",
				Query:       "somepath(?targetthree, ?dependencythree)",
				Verb:        "aquery",
			},
		}
		cmd := &cobra.Command{Use: "aquery"}
		err := q.Run(cmd, []string{})
		g.Expect(err).To(BeNil())
	})
}
