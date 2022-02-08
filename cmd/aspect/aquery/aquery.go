/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package aquery

import (
	"context"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/aquery"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultAQueryCmd() *cobra.Command {
	return NewAQueryCommand(ioutils.DefaultStreams, bazel.New())
}

func NewAQueryCommand(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	cmd := &cobra.Command{
		Use:                "aquery",
		Short:              "Executes an aquery.",
		Long:               "Executes a query language expression over a specified subgraph of the build dependency graph using aquery.",
		DisableFlagParsing: true,
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				interceptors.WorkspaceRootInterceptor(),
				interceptors.BazelFlagInterceptor(),
				flags.FlagsInterceptor(streams),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				workspaceRoot := ctx.Value(interceptors.WorkspaceRootKey).(string)
				bzl.SetWorkspaceRoot(workspaceRoot)
				q := aquery.New(streams, bzl, true)
				return q.Run(cmd, args)
			},
		),
	}

	return cmd
}
