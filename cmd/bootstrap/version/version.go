package version

import (
	"fmt"

	"github.com/spf13/cobra"
)

func NewVersionCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "version",
		Short: "Print the version of aspect CLI as well as tools it invokes.",
		Long:  `Prints version info on colon-separated lines, just like bazel does`,
		Run: func(cmd *cobra.Command, args []string) {
			// TODO(chuck): IMPLEMENT ME!
			fmt.Println("Placeholder for version info.")
		},
		// RunE: interceptors.Run(
		// 	[]interceptors.Interceptor{
		// 		flags.FlagsInterceptor(streams),
		// 	},
		// 	func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
		// 		return v.Run(bzl)
		// 	},
		// ),
	}
}
