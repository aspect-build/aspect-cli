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

package cquery

import (
	"github.com/spf13/cobra"

	"github.com/aspect-build/aspect-cli/pkg/aspect/cquery"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/interceptors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

func NewDefaultCmd() *cobra.Command {
	return NewCQueryCommand(ioutils.DefaultStreams, bazel.WorkspaceFromWd)
}

func NewCQueryCommand(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "cquery [expression |  <preset name> [arg ...]]",
		Short: "Query the dependency graph, honoring configuration flags",
		Long: `Executes a query language expression over a specified subgraph of the configured build dependency graph.

cquery should be preferred over query for typical usage, since it includes the analysis phase and
therefore provides results that match what the build command will do.

Note that cquery is especially powerful as the graph can be processed by a purpose-built program
written in Starlark. See <https://bazel.build/query/cquery#output-format-definition>.

Aspect CLI introduces the second form, where in place of an expression, you can give a preset query name.
Some preset queries also accept parameters, such as labels of targets, which can be provided as arguments.
If they are absent and the session is interactive, the user will be prompted to supply these.

Documentation: <https://bazel.build/query/cquery>
`,
		// Note, we should cquery in the "common" commands rather than query, because most users
		// ought to use cquery most of the time.
		GroupID: "common",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			cquery.New(streams, bzl, true).Run,
		),
	}

	return cmd
}
