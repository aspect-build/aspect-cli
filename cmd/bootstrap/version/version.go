package version

import (
	"os"

	"aspect.build/cli/buildinfo"
	"aspect.build/cli/pkg/versionwriter"
	"github.com/spf13/cobra"
)

func NewVersionCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "version",
		Short: "Print the version of aspect CLI as well as tools it invokes.",
		Long:  `Prints version info on colon-separated lines, just like bazel does`,
		RunE: func(cmd *cobra.Command, args []string) error {
			bi := buildinfo.Current()
			vw := versionwriter.NewFromBuildInfo("Aspect", *bi, versionwriter.Conventional)
			if _, err := vw.Print(os.Stdout); err != nil {
				return err
			}
			return nil
		},
	}
}
