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

	"aspect.build/cli/pkg/aspect/query"
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
			Spawn([]string{"query", "somepath(//cmd/aspect/query:query, @com_github_bazelbuild_bazelisk//core:go_default_library)"}, streams).
			Return(0, nil)

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

		cmd := &cobra.Command{Use: "query"}
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
			Spawn([]string{"query", "somepath(//cmd/aspect/query:query, @com_github_bazelbuild_bazelisk//core:go_default_library)"}, streams).
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
		cmd := &cobra.Command{Use: "query"}
		err = q.Run(cmd, []string{"why"})
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

		cmd := &cobra.Command{Use: "query"}
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
			Spawn([]string{"query", "somepath(//cmd/aspect/query:query, @com_github_bazelbuild_bazelisk//core:go_default_library)"}, streams).
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
		cmd := &cobra.Command{Use: "query"}
		err = q.Run(cmd, []string{})
		g.Expect(err).To(BeNil())
	})

	t.Run("user defined queries can overwrite default predefined queries", func(t *testing.T) {
		g := NewGomegaWithT(t)

		viper := *viper.New()
		cfg, err := os.CreateTemp(os.Getenv("TEST_TMPDIR"), "cfg***.ini")

		g.Expect(err).To(BeNil())

		viper.SetConfigFile(cfg.Name())
		viper.Set("query.presets.why.description", "Override the default why verb. Determine why targetA depends on targetB")
		viper.Set("query.presets.why.query", "somepath(?targetA, ?targetB)")
		viper.Set("query.presets.why.verb", "query")

		result := shared.PrecannedQueries("query", viper)
		g.Expect(len(result)).To(Equal(2))

		g.Expect(result[0].Description).To(Equal("Get the deps of a target"))
		g.Expect(result[0].Query).To(Equal("deps(?target)"))
		g.Expect(result[0].Verb).To(Equal("query"))
		g.Expect(result[0].Name).To(Equal("deps"))

		g.Expect(result[1].Description).To(Equal("Override the default why verb. Determine why targetA depends on targetB"))
		g.Expect(result[1].Query).To(Equal("somepath(?targetA, ?targetB)"))
		g.Expect(result[1].Verb).To(Equal("query"))
		g.Expect(result[1].Name).To(Equal("why"))
	})
}
