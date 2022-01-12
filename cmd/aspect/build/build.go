/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package build

import (
	"context"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/build"
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
		bazel.New(),
	)
}

// NewBuildCmd creates a new build cobra command.
func NewBuildCmd(
	streams ioutils.Streams,
	pluginSystem system.PluginSystem,
	bzl bazel.Bazel,
) *cobra.Command {
	return &cobra.Command{
		Use:   "build",
		Short: "Builds the specified targets, using the options.",
		Long: "Invokes bazel build on the specified targets. " +
			"See 'bazel help target-syntax' for details and examples on how to specify targets to build.",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				interceptors.WorkspaceRootInterceptor(),
				pluginSystem.BESBackendInterceptor(),
				pluginSystem.BuildHooksInterceptor(streams),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				workspaceRoot := ctx.Value(interceptors.WorkspaceRootKey).(string)
				bzl.SetWorkspaceRoot(workspaceRoot)
				b := build.New(streams, bzl)
				besBackend := ctx.Value(system.BESBackendInterceptorKey).(bep.BESBackend)
				return b.Run(args, besBackend)
			},
		),
	}
}
