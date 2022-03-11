/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
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
			[]interceptors.Interceptor{},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				q := query.New(streams, bzl, true)
				return q.Run(cmd, args)
			},
		),
	}

	return cmd
}
