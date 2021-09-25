/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package clean

import (
	"fmt"
	"os"

	"github.com/manifoldco/promptui"
	"github.com/mattn/go-isatty"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

const (
	skipPromptKey = "clean.skip_prompt"
)

// Clean represents the aspect clean command.
type Clean struct {
	ioutils.Streams
	bzl bazel.Spawner

	Expunge      bool
	ExpungeAsync bool
}

// New creates a Clean command.
func New(
	streams ioutils.Streams,
	bzl bazel.Spawner,
) *Clean {
	return &Clean{
		Streams: streams,
		bzl:     bzl,
	}
}

// Run runs the aspect build command.
func (c *Clean) Run(_ *cobra.Command, _ []string) error {
	skip := viper.GetBool(skipPromptKey)
	interactive := isatty.IsTerminal(os.Stdout.Fd()) || isatty.IsCygwinTerminal(os.Stdout.Fd())
	if interactive && !skip {
		choose := promptui.Select{
			Label: "Clean can have a few behaviors. Which do you want?",
			Items: []string{
				"Reclaim disk space for this workspace (same as bazel clean)",
				"Reclaim disk space for all Bazel workspaces",
				"Prepare to perform a non-incremental build",
				"Invalidate all repository rules, causing them to recreate external repos",
				"Workaround inconsistent state in the output tree",
			},
		}

		i, _, err := choose.Run()

		if err != nil {
			return fmt.Errorf("prompt failed: %v", err)
		}

		switch i {

		case 0:
			// Allow user to opt-out of our fancy "clean" command and just behave like bazel
			fmt.Println("You can skip this prompt to make 'aspect clean' behave the same as 'bazel clean'")
			remember := promptui.Prompt{
				Label:     "Remember this choice and skip the prompt in the future",
				IsConfirm: true,
			}
			_, err := remember.Run()
			if err == nil {
				viper.Set(skipPromptKey, "true")
				if err := viper.WriteConfig(); err != nil {
					return fmt.Errorf("failed to update config file: %v", err)
				}
			}
		case 1:
			fmt.Printf("Sorry, this is not implemented yet: discover all bazel workspaces on the machine")
			return nil
		case 2:
			fmt.Println("It's faster to perform a non-incremental build by choosing a different output base.")
			fmt.Println("Instead of running 'clean' you should use the --output_base flag.")
			fmt.Println("Run 'aspect help clean' for more info.")
			return nil
		case 3:
			fmt.Println("It's faster to invalidate repository rules by using the sync command.")
			fmt.Println("Instead of running 'clean' you should run 'aspect sync --configure'")
			fmt.Println("Run 'aspect help clean' for more info.")
			return nil
		case 4:
			fmt.Println("Bazel is a correct build tool, and it should not be possible to get inconstent state.")
			fmt.Println("We highly recommend you file a bug reporting this problem so that the offending rule")
			fmt.Println("implementation can be fixed.")
			workaround := promptui.Prompt{
				Label:     "Temporarily workaround the bug by deleting the output folder",
				IsConfirm: true,
			}
			_, err := workaround.Run()
			if err != nil {
				return fmt.Errorf("prompt failed: %v", err)
			}
		}
	}

	cmd := []string{"clean"}
	if c.Expunge {
		cmd = append(cmd, "--expunge")
	}
	if c.ExpungeAsync {
		cmd = append(cmd, "--expunge_async")
	}
	if exitCode, err := c.bzl.Spawn(cmd); exitCode != 0 {
		err = &aspecterrors.ExitError{
			Err:      err,
			ExitCode: exitCode,
		}
		return err
	}

	return nil
}
