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
	"github.com/spf13/cobra"

	"github.com/aspect-build/aspect-cli/pkg/aspect/coverage"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/hints"
	"github.com/aspect-build/aspect-cli/pkg/interceptors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/aspect-build/aspect-cli/pkg/plugin/system"
)

// NewDefaultCmd creates a new coverage cobra command with the default
// dependencies.
func NewDefaultCmd(pluginSystem system.PluginSystem) *cobra.Command {
	return NewCmd(
		ioutils.DefaultStreams,
		hints.DefaultStreams,
		pluginSystem,
		bazel.WorkspaceFromWd,
	)
}

func NewCmd(
	streams ioutils.Streams,
	hstreams ioutils.Streams,
	pluginSystem system.PluginSystem,
	bzl bazel.Bazel,
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

Read [the Bazel coverage documentation](https://bazel.build/configure/coverage) on gathering code coverage data.

See 'aspect help target-syntax' for details and examples on how to specify targets.
`,
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
				pluginSystem.BESBackendInterceptor(),
				pluginSystem.TestHooksInterceptor(streams),
			},
			coverage.New(streams, hstreams, bzl).Run,
		),
	}
}
