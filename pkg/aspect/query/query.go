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

package query

import (
	"fmt"

	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	"aspect.build/cli/pkg/aspect/query/shared"
	"aspect.build/cli/pkg/aspect/root/config"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

const (
	useCQuery               = "query.cquery.use"
	useCQueryInquired       = "query.cquery.inquired"
	allowAllQueries         = "query.all.allow"
	allowAllQueriesInquired = "query.all.inquired"
)

type Query struct {
	ioutils.Streams

	Bzl           bazel.Bazel
	IsInteractive bool

	Presets []*shared.PresetQuery
	Prefs   viper.Viper

	Prompt       func(label string) shared.PromptRunner
	Confirmation func(question string) shared.ConfirmationRunner
	Select       func(presetNames []string) shared.SelectRunner
}

func New(streams ioutils.Streams, bzl bazel.Bazel, isInteractive bool) *Query {
	runner := *viper.GetViper()

	// the list of available preset queries will potentially be updated during the "Run" function.
	// if the user requests that query also show aquery and cquery predefined queries then these
	// will be added to the list of presets
	presets := shared.PrecannedQueries("query", runner)

	return &Query{
		Streams:       streams,
		Bzl:           bzl,
		IsInteractive: isInteractive,
		Presets:       presets,
		Prompt:        shared.Prompt,
		Select:        shared.Select,
		Confirmation:  shared.Confirmation,
		Prefs:         runner,
	}
}

func (runner *Query) Run(cmd *cobra.Command, args []string) error {
	nonFlags, flags, err := bazel.ParseOutBazelFlags(cmd.CalledAs(), args)
	if err != nil {
		return err
	}

	if len(nonFlags) == 0 {
		// Only check the query configuration if user calls the command with no arguments
		// so we don't go interactive when a user runs `aspect [ac]query <expression>` after
		// installing aspect
		err := runner.checkConfig(
			allowAllQueries,
			allowAllQueriesInquired,
			"Include predefined aquery's and cquery's when calling query",
		)
		if err != nil {
			return err
		}

		err = runner.checkConfig(
			useCQuery,
			useCQueryInquired,
			"Use cquery instead of query",
		)
		if err != nil {
			return err
		}
	}

	command := "query"

	if runner.Prefs.GetBool(useCQuery) {
		command = "cquery"
	}

	if runner.Prefs.GetBool(allowAllQueries) {
		runner.Presets = shared.PrecannedQueries("", runner.Prefs)
	}

	presets, presetNames, err := shared.ProcessQueries(runner.Presets)
	if err != nil {
		return shared.GetPrettyError(cmd, err)
	}

	command, query, runReplacements, err := shared.SelectQuery(command, presets, runner.Presets, presetNames, runner.Streams, nonFlags, runner.Select)
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

func (runner *Query) checkConfig(baseUseKey string, baseInquiredKey string, question string) error {
	if !runner.Prefs.GetBool(baseInquiredKey) {
		runner.Prefs.Set(baseInquiredKey, true)

		// Y = no error; N = error
		_, err := runner.Confirmation(question).Run()

		configFile, created, err := config.SetInHomeConfig(baseUseKey, err == nil)
		if err != nil {
			return err
		}
		_, _, err = config.SetInHomeConfig(baseInquiredKey, true)
		if err != nil {
			return err
		}
		if created {
			fmt.Printf("Created %s\n", configFile)
		} else {
			fmt.Printf("Updated %s\n", configFile)
		}
	}

	return nil
}
