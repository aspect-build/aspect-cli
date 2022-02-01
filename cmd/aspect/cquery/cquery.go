/*
Copyright © 2021 Aspect Build Systems

Not licensed for re-use
*/

package cquery

import (
	"context"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/cquery"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultCQueryCmd() *cobra.Command {
	return NewCQueryCommand(ioutils.DefaultStreams, bazel.New())
}

func NewCQueryCommand(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "cquery",
		Short: "Executes a cquery.",
		Long:  "Executes a query language expression over a specified subgraph of the build dependency graph using cquery.",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				interceptors.WorkspaceRootInterceptor(),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				workspaceRoot := ctx.Value(interceptors.WorkspaceRootKey).(string)
				bzl.SetWorkspaceRoot(workspaceRoot)
				q := cquery.New(streams, bzl, true)
				return q.Run(cmd, args)
			},
		),
	}

	return cmd
}
