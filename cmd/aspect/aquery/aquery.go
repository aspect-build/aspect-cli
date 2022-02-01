/*
Copyright © 2021 Aspect Build Systems

Not licensed for re-use
*/

package aquery

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/aquery"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultAQueryCmd() *cobra.Command {
	return NewAQueryCommand(ioutils.DefaultStreams, bazel.New())
}

func NewAQueryCommand(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	q := aquery.New(streams, bzl, true)

	cmd := &cobra.Command{
		Use:   "aquery",
		Short: "Executes an aquery.",
		Long:  "Executes a query language expression over a specified subgraph of the build dependency graph using aquery.",
		RunE:  q.Run,
	}

	return cmd
}
