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

func NewQueryCommand(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	q := query.New(streams, bzl, true)

	// TODO: Queries should be loadable from the plugin config
	// https://github.com/aspect-build/aspect-cli/issues/98
	q.Presets = []*query.PresetQuery{
		{
			Name:        "why",
			Description: "Determine why targetA depends on targetB",
			Query:       "somepath(?targetA, ?targetB)",
		},
	}

	cmd := &cobra.Command{
		Use:   "query",
		Short: "Executes a dependency graph query.",
		Long:  "Executes a query language expression over a specified subgraph of the build dependency graph.",
		RunE:  q.Run,
	}

	cmd.PersistentFlags().BoolVarP(&q.ShowGraph, "show_graph", "", false, `Shows a graph for of the result`)

	return cmd
}
