/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package build

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/build"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

// NewDefaultBuildCmd creates a new build cobra command with the default
// dependencies.
func NewDefaultBuildCmd() *cobra.Command {
	return NewBuildCmd(ioutils.DefaultStreams, bazel.New())
}

// NewBuildCmd creates a new build cobra command.
func NewBuildCmd(
	streams ioutils.Streams,
	bzl bazel.Spawner,
) *cobra.Command {
	b := build.New(streams, bzl)

	cmd := &cobra.Command{
		Use:   "build",
		Short: "Invoke a build on targets",
		Long:  "Invokes bazel build on provided targets",
		RunE:  b.Run,
	}

	return cmd
}
