/*
Copyright © 2021 Aspect Build Systems

Not licensed for re-use
*/

package query

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/query"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultQueryCmd() *cobra.Command {
	return NewQueryCommand(ioutils.DefaultStreams, bazel.New())
}

func NewQueryCommand(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	q := query.New(streams, bzl, true)

	cmd := &cobra.Command{
		Use:   "query",
		Short: "Executes a dependency graph query.",
		Long:  "Executes a query language expression over a specified subgraph of the build dependency graph.",
		RunE:  q.Run,
	}

	return cmd
}
