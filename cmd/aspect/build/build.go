/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package build

import (
	"errors"
	"fmt"

	"github.com/spf13/cobra"

	rootFlags "aspect.build/cli/cmd/aspect/root/flags"
	"aspect.build/cli/pkg/aspect/build"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/pathutils"
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
	cmd := &cobra.Command{
		Use:   "build",
		Short: "Builds the specified targets, using the options.",
		Long: "Invokes bazel build on the specified targets. " +
			"See 'bazel help target-syntax' for details and examples on how to specify targets to build.",
		RunE: pathutils.InvokeCmdInsideWorkspace(func(workspaceRoot string, cmd *cobra.Command, args []string) (exitErr error) {
			isInteractiveMode, err := cmd.Root().PersistentFlags().GetBool(rootFlags.InteractiveFlagName)
			if err != nil {
				return err
			}

			// TODO(f0rmiga): test this post-build hook.
			defer func() {
				errs := pluginSystem.ExecutePostBuild(isInteractiveMode).Errors()
				if len(errs) > 0 {
					for _, err := range errs {
						fmt.Fprintf(streams.Stderr, "Error: failed to run build command: %v\n", err)
					}
					var err *aspecterrors.ExitError
					if errors.As(exitErr, &err) {
						err.ExitCode = 1
					}
				}
			}()

			bzl.SetWorkspaceRoot(workspaceRoot)
			b := build.New(streams, bzl)
			return pluginSystem.WithBESBackend(cmd.Context(), func(besBackend bep.BESBackend) error {
				return b.Run(args, besBackend)
			})
		}),
	}

	return cmd
}
