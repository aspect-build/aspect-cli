/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package test

import (
	"context"

	"github.com/spf13/cobra"

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
		bazel.New(),
	)
}

func NewTestCmd(
	streams ioutils.Streams,
	pluginSystem system.PluginSystem,
	bzl bazel.Bazel,
) *cobra.Command {
	return &cobra.Command{
		Use:   "test",
		Short: "Builds the specified targets and runs all test targets among them.",
		Long: `Builds the specified targets and runs all test targets among them (test targets
might also need to satisfy provided tag, size or language filters) using
the specified options.

This command accepts all valid options to 'build', and inherits
defaults for 'build' from your .bazelrc.  If you don't use .bazelrc,
don't forget to pass all your 'build' options to 'test' too.

See 'bazel help target-syntax' for details and examples on how to
specify targets.
`,
		DisableFlagParsing: true,
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				interceptors.WorkspaceRootInterceptor(),
				interceptors.BazelFlagInterceptor(),
				pluginSystem.BESBackendInterceptor(),
				pluginSystem.TestHooksInterceptor(streams),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				workspaceRoot := ctx.Value(interceptors.WorkspaceRootKey).(string)
				bzl.SetWorkspaceRoot(workspaceRoot)
				t := test.New(streams, bzl)
				besBackend := ctx.Value(system.BESBackendInterceptorKey).(bep.BESBackend)
				return t.Run(args, besBackend)
			},
		),
	}
}
