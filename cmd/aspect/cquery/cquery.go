/*
Copyright © 2021 Aspect Build Systems

Not licensed for re-use
*/

package cquery

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/cquery"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultCQueryCmd() *cobra.Command {
	return NewCQueryCommand(ioutils.DefaultStreams, bazel.New())
}

func NewCQueryCommand(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	q := cquery.New(streams, bzl, true)

	cmd := &cobra.Command{
		Use:   "cquery",
		Short: "Executes a cquery.",
		Long:  "Executes a query language expression over a specified subgraph of the build dependency graph using cquery.",
		RunE:  q.Run,
	}

	return cmd
}
