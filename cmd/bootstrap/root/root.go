package root

import (
	"aspect.build/cli/cmd/bootstrap/initbzlwksp"
	"aspect.build/cli/cmd/bootstrap/version"
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
	}
	cmd.AddCommand(version.NewVersionCmd())
	cmd.AddCommand(initbzlwksp.NewInitCmd())
	return cmd
}
