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

package coverage

import (
	"context"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/coverage"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system"
	"aspect.build/cli/pkg/plugin/system/bep"
)

// NewDefaultCoverageCmd creates a new coverage cobra command with the default
// dependencies.
func NewDefaultCoverageCmd(pluginSystem system.PluginSystem) *cobra.Command {
	return NewCoverageCmd(
		ioutils.DefaultStreams,
		pluginSystem,
		bazel.FindFromWd,
	)
}

func NewCoverageCmd(
	streams ioutils.Streams,
	pluginSystem system.PluginSystem,
	bzlProvider bazel.BazelProvider,
) *cobra.Command {
	return &cobra.Command{
		Use:   "coverage --combined_report=<value> <target pattern> [<target pattern> ...]",
		Args:  cobra.MinimumNArgs(1),
		Short: "Same as 'test', but also generates a code coverage report.",
		Long: `To produce a coverage report, use bazel coverage --combined_report=lcov [target].
This runs the tests for the target, generating coverage reports in the lcov format for each file.

Once finished, bazel runs an action that collects all the produced coverage files,
and merges them into one, which is then finally created under
$(bazel info output_path)/_coverage/_coverage_report.dat.

Coverage reports are also produced if tests fail, though note that this does not extend to the
failed tests - only passing tests are reported.

More documentation on gathering code coverage data with Bazel:
<https://bazel.build/configure/coverage>

See 'aspect help target-syntax' for details and examples on how to specify targets.
`,
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
				pluginSystem.BESBackendInterceptor(),
				pluginSystem.TestHooksInterceptor(streams),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				bzl, err := bzlProvider()
				if err != nil {
					return err
				}
				t := coverage.New(streams, bzl)
				besBackend := bep.BESBackendFromContext(ctx)
				return t.Run(args, besBackend)
			},
		),
	}
}
