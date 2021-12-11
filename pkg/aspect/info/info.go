/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package info

import (
	"context"

	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspecterrors"
)

type Info struct {
	ioutils.Streams

	ShowMakeEnv bool
}

func New(streams ioutils.Streams) *Info {
	return &Info{
		Streams: streams,
	}
}

func (v *Info) Run(ctx context.Context, _ *cobra.Command, args []string) error {
	bazelCmd := []string{"info"}
	if v.ShowMakeEnv {
		// Propagate the flag
		bazelCmd = append(bazelCmd, "--show_make_env")
	}
	bazelCmd = append(bazelCmd, args...)
	bzl := bazel.New()
	workspaceRoot := ctx.Value(interceptors.WorkspaceRootKey).(string)
	bzl.SetWorkspaceRoot(workspaceRoot)

	if exitCode, err := bzl.Spawn(bazelCmd); exitCode != 0 {
		err = &aspecterrors.ExitError{
			Err:      err,
			ExitCode: exitCode,
		}
		return err
	}

	return nil
}
