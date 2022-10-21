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

package test

import (
	"context"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspect/test"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system"
	"aspect.build/cli/pkg/plugin/system/bep"
)

// NewDefaultTestCmd creates a new test cobra command with the default
// dependencies.
func NewDefaultTestCmd(pluginSystem system.PluginSystem) *cobra.Command {
	return NewTestCmd(
		ioutils.DefaultStreams,
		pluginSystem,
		bazel.FindFromWd,
	)
}

func NewTestCmd(
	streams ioutils.Streams,
	pluginSystem system.PluginSystem,
	bzlProvider bazel.BazelProvider,
) *cobra.Command {
	return &cobra.Command{
		Use:   "test [--build_tests_only] <target pattern> [<target pattern> ...]",
		Args:  cobra.MinimumNArgs(1),
		Short: "Build the specified targets and run all test targets among them",
		Long: `Runs test targets and reports the test results.

Documentation: <https://bazel.build/docs/user-manual#running-tests>

First, the targets are built. See 'aspect help build' for more about the phases of a build.
By default, any targets that match the pattern(s) are built, even if they are not needed as inputs
to any test target. Use ` + "`--build_tests_only`" + ` to avoid building these targets.

Targets may be filtered from the pattern. See <https://bazel.build/docs/user-manual#test-selection>:
- by size, using ` + "`--test_size_filters`" + ` often used to select only "unit tests"
- by timeout, using ` + "`--test_timeout_filters`" + ` often used to select only fast tests,
- by tag, using ` + "`--test_tag_filters`" + `
- by language, using ` + "`--test_lang_filters`" + ` though it only understands those built-in to Bazel.
  Follow https://github.com/bazelbuild/bazel/issues/12618

The tests are run following a well-specified contract between Bazel and the test runner process, see
<https://bazel.build/reference/test-encyclopedia>

This command accepts all valid options to 'build', and inherits
defaults for 'build' from your .bazelrc.  If you don't use .bazelrc,
don't forget to pass all your 'build' options to 'test' too.

See 'aspect help target-syntax' for details and examples on how to specify targets.
`,
		GroupID: "common",
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
				t := test.New(streams, bzl)
				besBackend := bep.BESBackendFromContext(ctx)
				return t.Run(args, besBackend)
			},
		),
	}
}
