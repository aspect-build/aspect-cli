/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package query

import (
	"context"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/query"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultQueryCmd() *cobra.Command {
	return NewQueryCommand(ioutils.DefaultStreams, bazel.FindFromWd)
}

func NewQueryCommand(streams ioutils.Streams, bzlProvider bazel.BazelProvider) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "query",
		Short: "Query the dependency graph, ignoring configuration flags",
		Long: `Executes a query language expression over a specified subgraph of the build dependency graph.

Note that this ignores the current configuration. Most users should use cquery instead,
unless you have a specific need to query the unconfigured graph.`,
		// Note: we list query in the "built-in" rather than "common" group because most users should
		// use cquery most of the time.
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				bzl, err := bzlProvider()
				if err != nil {
					return err
				}
				q := query.New(streams, bzl, true)
				return q.Run(cmd, args)
			},
		),
	}

	return cmd
}
