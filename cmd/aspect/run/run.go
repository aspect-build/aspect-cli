/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package run

import (
	"context"

	"github.com/spf13/cobra"

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
		bazel.New(),
	)
}

func NewRunCmd(
	streams ioutils.Streams,
	pluginSystem system.PluginSystem,
	bzl bazel.Bazel,
) *cobra.Command {
	return &cobra.Command{
		Use:   "run",
		Short: "Builds the specified target and runs it with the given arguments.",
		// TODO(f0rmiga): the following comment from 'bazel --help run' may not
		// be what we want to provide to our users.
		Long: `'run' accepts any 'build' options, and will inherit any defaults
provided by .bazelrc.

If your script needs stdin or execution not constrained by the bazel lock,
use 'bazel run --script_path' to write a script and then execute it.
`,
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				pluginSystem.BESBackendInterceptor(),
				pluginSystem.RunHooksInterceptor(streams),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				r := run.New(streams, bzl)
				besBackend := ctx.Value(system.BESBackendInterceptorKey).(bep.BESBackend)
				return r.Run(args, besBackend)
			},
		),
	}
}
