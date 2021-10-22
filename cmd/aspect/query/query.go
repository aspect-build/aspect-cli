/*
Copyright © 2021 Aspect Build Systems

Not licensed for re-use
*/

package query

import (
	"aspect.build/cli/pkg/aspect/query"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"github.com/spf13/cobra"
)

func NewDefaultQueryCmd() *cobra.Command {
	return NewQueryCommand(ioutils.DefaultStreams, bazel.New())
}

func NewQueryCommand(streams ioutils.Streams, bzl bazel.Spawner) *cobra.Command {
	q := query.New(streams, bzl, true)

	// Load these from somewhere
	q.Presets = []*query.PresetQuery{
		{
			Name:        "why",
			Description: "Determine why a target depends on another",
			Query:       "somepath(?target, ?dependency)",
		},
	}

	cmd := &cobra.Command{
		Use:   "query",
		Short: "Executes a dependency graph query.",
		RunE:  q.Run,
	}

	cmd.PersistentFlags().BoolVarP(&q.ShowGraph, "show_graph", "", false, `Shows a graph for of the result`)

	return cmd
}
