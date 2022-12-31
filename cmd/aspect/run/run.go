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

package run

import (
	"context"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspect/run"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system"
	"aspect.build/cli/pkg/plugin/system/bep"
)

// NewDefaultCmd creates a new run cobra command with the default
// dependencies.
func NewDefaultCmd(pluginSystem system.PluginSystem) *cobra.Command {
	return NewCmd(
		ioutils.DefaultStreams,
		pluginSystem,
		bazel.FindFromWd,
	)
}

func NewCmd(
	streams ioutils.Streams,
	pluginSystem system.PluginSystem,
	bzlProvider bazel.BazelProvider,
) *cobra.Command {
	return &cobra.Command{
		Use:   "run [--run_under=command-prefix] <target> -- [args for program ...]",
		Args:  cobra.MinimumNArgs(1),
		Short: "Build a single target and run it with the given arguments",
		Long: `Equivalent to ` + "`aspect build <target>`" + ` followed by spawning the resulting executable.

Documentation: <https://bazel.build/docs/user-manual#running-executables>

Two environment variables will be present that the program may reference:
- ` + "`BUILD_WORKSPACE_DIRECTORY`" + `: the root of the workspace where the build was run.
- ` + "`BUILD_WORKING_DIRECTORY`" + `: the current working directory where Bazel was run from.

Note that the ` + "`<target>`" + `may have an ` + "`args`" + ` and ` + "`env`" + `attribute. The ` + "`run`" + ` command honors these and
sets the arguments and environment of the spawned executable, unlike if the binary is executed as
an action during a build step, or is run directly outside of Bazel.
See <https://bazel.build/reference/be/common-definitions#common-attributes-binaries>.

` + "`run`" + ` accepts any ` + "`build`" + ` options, and will inherit any defaults provided by ` + "`.bazelrc.`" + `

If your script needs stdin or execution not constrained by the bazel lock,
use ` + "`bazel run --script_path`" + ` to write a script and then execute it.

By default, the program is run with a working directory inside $(aspect info execution_root).
Some programs expect to be run under a certain working directory, such as the workspace root.
Use the [--run_under](https://bazel.build/docs/user-manual#run_under) flag with a cd command, like
` + "`aspect run --run_under=\"cd $PWD &&\" //my:program`" + ` to use the current directory.
Another common approach if the program's code is in your repo (first-party) is to check for the
presence of ` + "`BUILD_WORKSPACE_DIRECTORY`" + ` in the environment, then change the working
directory of the process. You'd typically do this at the very beginning of the program execution.
`,
		GroupID:               "common",
		DisableFlagsInUseLine: true,
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
				pluginSystem.BESBackendInterceptor(),
				pluginSystem.RunHooksInterceptor(streams),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				bzl, err := bzlProvider()
				if err != nil {
					return err
				}
				r := run.New(streams, bzl)
				besBackend := bep.BESBackendFromContext(ctx)
				return r.Run(args, besBackend)
			},
		),
	}
}
