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

package build

import (
	"github.com/spf13/cobra"

	"github.com/aspect-build/aspect-cli/pkg/aspect/build"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/interceptors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/aspect-build/aspect-cli/pkg/plugin/system"
)

// NewDefaultCmd creates a new build cobra command with the default
// dependencies.
func NewDefaultCmd(pluginSystem system.PluginSystem) *cobra.Command {
	return NewCmd(
		ioutils.DefaultStreams,
		pluginSystem,
		bazel.WorkspaceFromWd,
	)
}

// NewCmd creates a new build cobra command.
func NewCmd(
	streams ioutils.Streams,
	pluginSystem system.PluginSystem,
	bzl bazel.Bazel,
) *cobra.Command {
	return &cobra.Command{
		Use:   "build <target patterns>",
		Args:  cobra.MinimumNArgs(1),
		Short: "Build the specified targets",
		Long: `Performs a build on the specified targets, producing their default outputs.

Documentation: <https://bazel.build/run/build#bazel-build>

Run 'aspect help target-syntax' for details and examples on how to specify targets to build.

Commonly used flags
-------------------

Bazel will first fetch any missing or out-of-date external dependencies.
You can run build with ` + "`--fetch=false`" + ` to inhibit this.
See 'aspect help fetch' for more information.

Since Bazel has no analyze command, you can use ` + "`build --nobuild`" + ` to only load and analyze
BUILD files without spawning any build actions. See https://github.com/bazelbuild/bazel/issues/15318

The build will halt as soon as the first error is encountered. Use ` + "`--keep_going (-k)`" + ` to
continue building.

Note that the rule implementation(s) may only run a subset of their actions to produce the default
outputs of the requested targets.
To create non-default outputs, consider using the ` + "`--output_groups`" + ` flag.

The target pattern may be further filtered using the flag
[--build_tag_filters](https://bazel.build/reference/command-line-reference#flag--build_tag_filters)
`,
		GroupID: "common",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
				pluginSystem.BESBackendInterceptor(),
				pluginSystem.BuildHooksInterceptor(streams),
			},
			build.New(streams, bzl).Run,
		),
	}
}
