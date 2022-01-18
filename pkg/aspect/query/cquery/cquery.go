/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package cquery

import (
	"github.com/spf13/cobra"

	shared "aspect.build/cli/pkg/aspect/query"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

type CQuery struct {
	ioutils.Streams

	Bzl           bazel.Bazel
	IsInteractive bool

	Presets []*shared.PresetQuery

	Prompt       func(label string) shared.PromptRunner
	Confirmation func(question string) shared.ConfirmationRunner
	Select       func(presetNames []string) shared.SelectRunner
}

func New(streams ioutils.Streams, bzl bazel.Bazel, isInteractive bool) *CQuery {
	presets := shared.GetPrecannedQueries("cquery")

	return &CQuery{
		Streams:       streams,
		Bzl:           bzl,
		IsInteractive: isInteractive,
		Presets:       presets,
		Prompt:        shared.Prompt,
		Select:        shared.Select,
		Confirmation:  shared.Confirmation,
	}
}

func (q *CQuery) Run(cmd *cobra.Command, args []string) error {
	verb := "cquery"
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
