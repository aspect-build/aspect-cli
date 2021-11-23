/*
Copyright © 2021 Aspect Build Systems

Not licensed for re-use
*/

package version

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/buildinfo"
	"aspect.build/cli/pkg/aspect/version"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultVersionCmd() *cobra.Command {
	return NewVersionCmd(ioutils.DefaultStreams)
}

func NewVersionCmd(streams ioutils.Streams) *cobra.Command {
	versionCmd := version.New(streams)

	versionCmd.BuildinfoRelease = buildinfo.Release
	versionCmd.BuildinfoGitStatus = buildinfo.GitStatus

	cmd := &cobra.Command{
		Use:   "version",
		Short: "Print the version of aspect CLI as well as tools it invokes.",
		Long:  `Prints version info on colon-separated lines, just like bazel does`,
		RunE:  versionCmd.Run,
	}

	cmd.PersistentFlags().BoolVarP(&versionCmd.GNUFormat, "gnu_format", "", false, "format space-separated following GNU convention")

	return cmd
}
