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
	v := *viper.GetViper()

	// the list of available preset queries will potentially be updated during the "Run" function.
	// if the user requests that query also show aquery and cquery predefined queries then these
	// will be added to the list of presets
	presets := shared.PrecannedQueries("query", v)

	return &Query{
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

func (q *Query) Run(cmd *cobra.Command, args []string) error {
	err := q.checkConfig(
		allowAllQueries,
		allowAllQueriesInquired,
		"Include predefined aquery's and cquery's when calling query",
	)
	if err != nil {
		return err
	}

	err = q.checkConfig(
		useCQuery,
		useCQueryInquired,
		"Use cquery instead of query",
	)
	if err != nil {
		return err
	}

	verb := cmd.Use

	if q.Prefs.GetBool(useCQuery) {
		verb = "cquery"
	}

	if q.Prefs.GetBool(allowAllQueries) {
		q.Presets = shared.PrecannedQueries("", q.Prefs)
	}

	presets, presetNames, err := shared.ProcessQueries(q.Presets)
	if err != nil {
		return shared.GetPrettyError(cmd, err)
	}

	presetVerb, query, runReplacements, err := shared.SelectQuery(verb, presets, q.Presets, presetNames, q.Streams, args, q.Select)

	if err != nil {
		return shared.GetPrettyError(cmd, err)
	}

	if runReplacements {
		query, err = shared.ReplacePlaceholders(query, args, q.Prompt)

		if err != nil {
			return shared.GetPrettyError(cmd, err)
		}
	}

	return shared.RunQuery(q.Bzl, presetVerb, query, q.Streams)
}

func (q *Query) checkConfig(baseUseKey string, baseInquiredKey string, question string) error {
	if !q.Prefs.GetBool(baseInquiredKey) {
		q.Prefs.Set(baseInquiredKey, true)

		// Y = no error; N = error
		_, err := q.Confirmation(question).Run()

		q.Prefs.Set(baseUseKey, err == nil)

		// TODO(chuck): FIX ME!
		if err := q.Prefs.WriteConfig(); err != nil {
			return fmt.Errorf("failed to update config file: %w", err)
		}
	}

	return nil
}
