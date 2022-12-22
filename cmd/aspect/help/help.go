package help

import (
	"aspect.build/cli/docs/help/topics"
	"github.com/spf13/cobra"
)

func NewCmd() *cobra.Command {
	cmd := cobra.Command{
		Use:  "help <command>",
		Args: cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {

			if len(args) == 1 {
				name := args[0]

				for _, cmd := range cmd.Root().Commands() {
					if cmd.Name() == name {
						return cmd.Help()
					}
				}

			}

			return cmd.Help()
		},
	}

	cmd.AddCommand(NewDefaultFlagsAsProtoCmd())

	// ### "Additional help topic commands" which are not runnable
	// https://pkg.go.dev/github.com/spf13/cobra#Command.IsAdditionalHelpTopicCommand
	cmd.AddCommand(&cobra.Command{
		Use:   "target-syntax",
		Short: "Explains the syntax for specifying targets.",
		Long:  topics.MustAssetString("target-syntax.md"),
	})
	cmd.AddCommand(&cobra.Command{
		Use:   "info-keys",
		Short: "Displays a list of keys used by the info command.",
		Long:  topics.MustAssetString("info-keys.md"),
	})
	cmd.AddCommand(&cobra.Command{
		Use:   "tags",
		Short: "Conventions for tags which are special.",
		Long:  topics.MustAssetString("tags.md"),
	})

	return &cmd
}
