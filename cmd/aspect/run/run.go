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

// NewDefaultRunCmd creates a new run cobra command with the default
// dependencies.
func NewDefaultRunCmd(pluginSystem system.PluginSystem) *cobra.Command {
	return NewRunCmd(
		ioutils.DefaultStreams,
		pluginSystem,
		bazel.FindFromWd,
	)
}

func NewRunCmd(
	streams ioutils.Streams,
	pluginSystem system.PluginSystem,
	bzlProvider bazel.BazelProvider,
) *cobra.Command {
	return &cobra.Command{
		Use:   "run",
		Short: "Build a single target and run it with the given arguments",
		// TODO(f0rmiga): the following comment from 'bazel --help run' may not
		// be what we want to provide to our users.
		Long: `'run' accepts any 'build' options, and will inherit any defaults
provided by .bazelrc.

If your script needs stdin or execution not constrained by the bazel lock,
use 'bazel run --script_path' to write a script and then execute it.
`,
		GroupID: "common",
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
