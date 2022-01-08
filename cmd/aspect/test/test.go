/*
Copyright © 2021 Aspect Build Systems

Not licensed for re-use
*/

package test

import (
	"context"
	"errors"
	"fmt"

	"github.com/spf13/cobra"

	rootFlags "aspect.build/cli/cmd/aspect/root/flags"
	"aspect.build/cli/pkg/aspect/test"
	"aspect.build/cli/pkg/aspecterrors"
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
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				interceptors.WorkspaceRootInterceptor(),
				pluginSystem.BESBackendInterceptor(),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				isInteractiveMode, err := cmd.Root().PersistentFlags().GetBool(rootFlags.InteractiveFlagName)
				if err != nil {
					return err
				}

				defer func() {
					errs := pluginSystem.ExecutePostTest(isInteractiveMode).Errors()
					if len(errs) > 0 {
						for _, err := range errs {
							fmt.Fprintf(streams.Stderr, "Error: failed to run test command: %v\n", err)
						}
						var err *aspecterrors.ExitError
						if errors.As(exitErr, &err) {
							err.ExitCode = 1
						}
					}
				}()

				workspaceRoot := ctx.Value(interceptors.WorkspaceRootKey).(string)
				bzl.SetWorkspaceRoot(workspaceRoot)
				t := test.New(streams, bzl)
				besBackend := ctx.Value(system.BESBackendInterceptorKey).(bep.BESBackend)
				return t.Run(args, besBackend)
			},
		),
	}
}
