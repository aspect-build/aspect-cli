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

package cquery

import (
	"context"

	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	"github.com/aspect-build/aspect-cli/pkg/aspect/query/shared"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

type CQuery struct {
	ioutils.Streams

	Bzl           bazel.Bazel
	IsInteractive bool

	Presets []*shared.PresetQuery
	Prefs   viper.Viper

	Prompt       func(label string) shared.PromptRunner
	Confirmation func(question string) shared.ConfirmationRunner
	Select       func(presetNames []string) shared.SelectRunner
}

func New(streams ioutils.Streams, bzl bazel.Bazel, isInteractive bool) *CQuery {
	v := *viper.GetViper()

	presets := shared.PrecannedQueries("cquery", v)

	return &CQuery{
		Streams:       streams,
		Bzl:           bzl,
		IsInteractive: isInteractive,
		Presets:       presets,
		Prompt:        shared.Prompt,
		Select:        shared.Select,
		Confirmation:  shared.Confirmation,
		Prefs:         v,
	}
}

func (runner *CQuery) Run(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
	nonFlags, flags, err := bazel.SeparateBazelFlags(cmd.CalledAs(), args)
	if err != nil {
		return err
	}

	presets, presetNames, err := shared.ProcessQueries(runner.Presets)
	if err != nil {
		return shared.GetPrettyError(cmd, err)
	}

	command, query, runReplacements, err := shared.SelectQuery(cmd.CalledAs(), presets, runner.Presets, presetNames, runner.Streams, nonFlags, flags, runner.Select)
	if err != nil {
		return shared.GetPrettyError(cmd, err)
	}

	if runReplacements {
		query, err = shared.ReplacePlaceholders(query, nonFlags, runner.Prompt)
		if err != nil {
			return shared.GetPrettyError(cmd, err)
		}

		return shared.RunQuery(runner.Bzl, command, runner.Streams, append(flags, query))
	} else {
		return shared.RunQuery(runner.Bzl, command, runner.Streams, args)
	}
}
