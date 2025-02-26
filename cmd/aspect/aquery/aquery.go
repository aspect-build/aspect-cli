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

package aquery

import (
	"github.com/spf13/cobra"

	"github.com/aspect-build/aspect-cli/pkg/aspect/aquery"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/interceptors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

func NewDefaultCmd() *cobra.Command {
	return NewAQueryCommand(ioutils.DefaultStreams, bazel.WorkspaceFromWd)
}

func NewAQueryCommand(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "aquery [expression |  <preset name> [arg ...]]",
		Short: "Query the action graph",
		Long: `Executes a query language expression over a specified subgraph of the action graph.

Read [the Bazel aquery documentation](https://bazel.build/query/aquery)

Aspect CLI introduces the second form, where in place of an expression, you can give a preset query name.
Some preset queries also accept parameters, such as labels of targets, which can be provided as arguments.
If they are absent and the session is interactive, the user will be prompted to supply these.`,
		Example: `# Get the action graph generated while building //src/target_a
$ aspect aquery '//src/target_a'

# Get the action graph generated while building all dependencies of //src/target_a
$ aspect aquery 'deps(//src/target_a)'

# Get the action graph generated while building all dependencies of //src/target_a
# whose inputs filenames match the regex ".*cpp".
$ aspect aquery 'inputs(".*cpp", deps(//src/target_a))'`,
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			aquery.New(streams, bzl, true).Run,
		),
	}

	return cmd
}
