package cmd

import (
	"fmt"
	"os"
	"path"

	aspect_wspace "aspect.build/cli/wspace"

	"github.com/bazelbuild/buildtools/wspace"
	"github.com/manifoldco/promptui"
	"github.com/spf13/cobra"
)

// initCmd represents the init command
var initCmd = &cobra.Command{
	Use:   "init",
	Short: "Setup Aspect and Bazel in the current folder",
	Long:  ``,
	Run: func(cmd *cobra.Command, args []string) {
		wd, _ := os.Getwd()
		root, _ := wspace.FindWorkspaceRoot(wd)
		if root != "" {
			fmt.Printf("Found existing Bazel workspace in %s from %s\n", root, wd)
			// check if aspect is installed there
		} else {
			fmt.Printf("No Bazel workspace found containing %s", wd)
			prompt := promptui.Prompt{
				Label:     "No Bazel workspace found in current folder. Create one",
				IsConfirm: true,
			}

			_, err := prompt.Run()

			if err != nil {
				fmt.Println("aspect init aborted")
				return
			}

			prompt = promptui.Prompt{
				Label:    "Name for the new Bazel workspace?",
				Validate: aspect_wspace.Validate,
				Default:  path.Base(wd),
			}

			wkspName, err := prompt.Run()

			if err != nil {
				fmt.Println("aspect init aborted")
				return
			}

			aspect_wspace.CreateWorkspace(wd, wkspName)
		}
	},
}

func init() {
	rootCmd.AddCommand(initCmd)
}
