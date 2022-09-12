package version

import (
	"strings"

	"aspect.build/cli/buildinfo"
	"github.com/spf13/cobra"
)

func NewVersionCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "version",
		Short: "Print the version of aspect CLI as well as tools it invokes.",
		Long:  `Prints version info on colon-separated lines, just like bazel does`,
		Run: func(cmd *cobra.Command, args []string) {
			var versionBuilder strings.Builder
			if buildinfo.Release != "" {
				versionBuilder.WriteString(buildinfo.Release)
				if buildinfo.GitStatus != "clean" {
					versionBuilder.WriteString(" (with local changes)")
				}
			} else {
				versionBuilder.WriteString("unknown [not built with --stamp]")
			}
		},
	}
}
