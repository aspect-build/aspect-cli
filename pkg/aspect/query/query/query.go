/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package query

import (
	"fmt"

	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	shared "aspect.build/cli/pkg/aspect/query"
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

	// will potentially be updated during Run() if the user requests that query also show aquery and cquery predefined queries
	presets := shared.GetPrecannedQueries("query", v)

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

	verb := "query"

	if q.Prefs.GetBool(useCQuery) {
		verb = "cquery"
	}

	if q.Prefs.GetBool(allowAllQueries) {
		q.Presets = shared.GetPrecannedQueries("", q.Prefs)
	}

	presets, presetNames, err := shared.ProcessQueries(q.Presets)
	if err != nil {
		return shared.GetPrettyError(cmd, err)
	}

	verb, query, runReplacements, err := shared.SelectQuery(verb, presets, q.Presets, presetNames, q.Streams, args, q.Select)

	if err != nil {
		return shared.GetPrettyError(cmd, err)
	}

	if runReplacements {
		query, err = shared.ReplacePlaceholders(query, args, q.Prompt)

		if err != nil {
			return shared.GetPrettyError(cmd, err)
		}
	}

	return shared.RunQuery(q.Bzl, verb, query)
}

func (q *Query) checkConfig(baseUseKey string, baseInquiredKey string, question string) error {
	if !q.Prefs.GetBool(baseInquiredKey) {
		q.Prefs.Set(baseInquiredKey, true)

		// if user types in y or Y then err will be nil. Any other input will not result in nil
		_, someErr := q.Confirmation(question).Run()

		q.Prefs.Set(baseUseKey, someErr == nil)

		if err := q.Prefs.WriteConfig(); err != nil {
			return fmt.Errorf("failed to update config file: %w", err)
		}
	}

	return nil
}
