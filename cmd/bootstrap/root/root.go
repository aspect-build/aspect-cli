package root

import (
	"fmt"

	"github.com/fatih/color"
	"github.com/spf13/cobra"
)

var (
	boldCyan = color.New(color.FgCyan, color.Bold)
)

func NewRootCmd() *cobra.Command {
	// Clients should not distinguish between `bootstrap` and `aspect-cli`.
	cmd := &cobra.Command{
		Use:           "aspect",
		Short:         "Aspect.build bazel wrapper",
		SilenceUsage:  true,
		SilenceErrors: true,
		Long:          boldCyan.Sprintf(`Aspect CLI`) + ` is a better frontend for running bazel`,
		// Suppress timestamps in generated Markdown, for determinism
		DisableAutoGenTag: true,
		Run: func(cmd *cobra.Command, args []string) {
			fmt.Println("Hello, World!")
		},
	}
	return cmd
}
