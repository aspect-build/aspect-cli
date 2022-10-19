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

package modquery

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/modquery"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultModQueryCmd() *cobra.Command {
	return NewModQueryCmd(ioutils.DefaultStreams)
}

func NewModQueryCmd(streams ioutils.Streams) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "modquery <query_type> [<args> ...]",
		Short: "Query the Bzlmod external dependency graph",
		Long: `The command will display a dependency tree or parts of the dependency tree, structured to display different kinds of insights depending on the query type.
Calling the command with no argument will default to:
bazel modquery tree root

<query_type> [<args> ...] can be one of the following:

    - tree: Displays the full dependency tree. Use the --from option to specify which module(s) you want the tree to start from (defaults to root which displays the whole dependency tree).
    - deps <module(s)>: Displays the direct dependencies of the target module(s).
    - path <module(s)_to>: Displays the shortest path found in the dependency graph from (any of) the --from module(s) to (any of) <module(s)_to>.
    - all_paths <module(s)_to>: Display the dependency graph starting from (any of) the --from module(s) and containing any existing paths to (any of) the <module(s)_to>.
    - explain <module(s)>: Prints all the places where the module is (or was) requested as a direct dependency, along with the reason why the respective final version was selected. It will display a pruned version of the all_paths root <module(s)> command which only contains the direct deps of the root, the <module(s)> leaves, along with their dependants (can be modified with --depth).
    - show <module(s)>: Prints the rule that generated these modules? repos (i.e. http_archive()).

<module> arguments must be of type:

    - root: The current (root) module you are inside of.
    - <name>@<version>: A specific module version.
    - <name>@_: Specifies the empty version of a module (for non-registry overridden) modules).
    - <name>: Can be used as a placeholder for all the present versions of the module <name>.
    - <repo_name>: The repo_name of one of the root project?s direct dependencies, as it is defined in the MODULE.bazel file.

<modules> means:

    - <module>,<module>,... : A list of comma separated modules, where each <module> has the form of one of the above.

NOTE: This command is still very experimental and the precise semantics
will change in the near future.`,
		GroupID: "built-in",
		Hidden:  true, // This command is documented as "very experimental"
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			modquery.New(streams).Run,
		),
	}

	return cmd
}
