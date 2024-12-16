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
	"github.com/spf13/cobra"

	"github.com/aspect-build/aspect-cli/pkg/aspect/query"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/interceptors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

func NewDefaultCmd() *cobra.Command {
	return NewQueryCommand(ioutils.DefaultStreams, bazel.WorkspaceFromWd)
}

func NewQueryCommand(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "query [expression |  <preset name> [arg ...]]",
		Short: "Query the dependency graph, ignoring configuration flags",
		Long: `Executes a query language expression over a specified subgraph of the unconfigured build dependency graph.

Note that this ignores the current configuration. Most users should use cquery instead,
unless you have a specific need to query the unconfigured graph.

Documentation: <https://bazel.build/query/quickstart>`,
		// Note: we list query in the "built-in" rather than "common" group because most users should
		// use cquery most of the time.
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			query.New(streams, bzl, true).Run,
		),
	}

	return cmd
}
