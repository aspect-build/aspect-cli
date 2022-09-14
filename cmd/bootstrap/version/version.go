package version

import (
	"fmt"

	"aspect.build/cli/buildinfo"
	"github.com/spf13/cobra"
)

func NewVersionCmd() *cobra.Command {
	gnuFormat := false
	cmd := &cobra.Command{
		Use:   "version",
		Short: "Print the version of aspect CLI as well as tools it invokes.",
		Long:  `Prints version info on colon-separated lines, just like bazel does`,
		RunE: func(cmd *cobra.Command, args []string) error {
			bi := buildinfo.Current()
			// format := versionwriter.Conventional
			// if gnuFormat {
			// 	format = versionwriter.GNU
			// }
			// vw := versionwriter.NewFromBuildInfo("Aspect", *bi, format)
			// if _, err := vw.Print(os.Stdout); err != nil {
			// 	return err
			// }

			format := buildinfo.ConventionalFormat
			if gnuFormat {
				format = buildinfo.GNUFormat
			}
			version := bi.UtilityVersion("Aspect", format)
			if _, err := fmt.Println(version); err != nil {
				return err
			}
			return nil
		},
	}
	cmd.PersistentFlags().BoolVarP(
		&gnuFormat,
		"gnu_format",
		"",
		false,
		"format space-separated following GNU convention",
	)
	return cmd
}
