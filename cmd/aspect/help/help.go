package help

import (
	"fmt"

	"github.com/spf13/cobra"
)

func NewCmd() *cobra.Command {
	cmd := cobra.Command{
		Use: "help <command>",
		RunE: func(cmd *cobra.Command, args []string) error {

			if len(args) == 0 {
				// `aspect help` with no args should display root command help string
				// (same as if you run `aspect` with no args)
				return cmd.Root().Help()
			} else if len(args) == 1 {
				name := args[0]

				for _, cmd := range cmd.Root().Commands() {
					if cmd.Name() == name {
						return cmd.Help()
					}
				}

				return fmt.Errorf("%s is not a known command", name)
			}

			return fmt.Errorf("You must specify exactly one command")
		},
	}

	cmd.AddCommand(NewDefaultFlagsAsProtoCmd())

	return &cmd
}
