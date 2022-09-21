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
	"context"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/build"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system"
	"aspect.build/cli/pkg/plugin/system/bep"
)

// NewDefaultBuildCmd creates a new build cobra command with the default
// dependencies.
func NewDefaultBuildCmd(pluginSystem system.PluginSystem) *cobra.Command {
	return NewBuildCmd(
		ioutils.DefaultStreams,
		pluginSystem,
		bazel.FindFromWd,
	)
}

// NewBuildCmd creates a new build cobra command.
func NewBuildCmd(
	streams ioutils.Streams,
	pluginSystem system.PluginSystem,
	bzlProvider bazel.BazelProvider,
) *cobra.Command {
	return &cobra.Command{
		Use:   "build",
		Short: "Builds the specified targets, using the options.",
		Long: "Invokes bazel build on the specified targets. " +
			"See 'bazel help target-syntax' for details and examples on how to specify targets to build.",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
				pluginSystem.BESBackendInterceptor(),
				pluginSystem.BuildHooksInterceptor(streams),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				bzl, err := bzlProvider()
				if err != nil {
					return err
				}
				b := build.New(streams, bzl)
				besBackend := bep.BESBackendFromContext(ctx)
				return b.Run(args, besBackend)
			},
		),
	}
}
