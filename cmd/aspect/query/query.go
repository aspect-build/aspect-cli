/*
Copyright © 2021 Aspect Build Systems

Not licensed for re-use
*/

package query

import (
	"context"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/query"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultQueryCmd() *cobra.Command {
	return NewQueryCommand(ioutils.DefaultStreams, bazel.New())
}

func NewQueryCommand(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "query",
		Short: "Executes a dependency graph query.",
		Long:  "Executes a query language expression over a specified subgraph of the build dependency graph.",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				interceptors.WorkspaceRootInterceptor(),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				workspaceRoot := ctx.Value(interceptors.WorkspaceRootKey).(string)
				bzl.SetWorkspaceRoot(workspaceRoot)
				q := query.New(streams, bzl, true)
				return q.Run(cmd, args)
			},
		),
	}

	return cmd
}
