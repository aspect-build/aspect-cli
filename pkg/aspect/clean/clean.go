/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package clean

import (
	"fmt"

	"github.com/manifoldco/promptui"
	"github.com/spf13/viper"

	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

const (
	skipPromptKey = "clean.skip_prompt"

	ReclaimOption         = "Reclaim disk space for this workspace (same as bazel clean)"
	ReclaimAllOption      = "Reclaim disk space for all Bazel workspaces"
	NonIncrementalOption  = "Prepare to perform a non-incremental build"
	InvalidateReposOption = "Invalidate all repository rules, causing them to recreate external repos"
	WorkaroundOption      = "Workaround inconsistent state in the output tree"

	outputBaseHint = `It's faster to perform a non-incremental build by choosing a different output base.
Instead of running 'clean' you should use the --output_base flag.
Run 'aspect help clean' for more info.
`
	syncHint = `It's faster to invalidate repository rules by using the sync command.
Instead of running 'clean' you should run 'aspect sync --configure'
Run 'aspect help clean' for more info.
`
	fileIssueHint = `Bazel is a correct build tool, and it should not be possible to get inconstent state.
We highly recommend you file a bug reporting this problem so that the offending rule
implementation can be fixed.
`

	rememberLine1 = "You can skip this prompt to make 'aspect clean' behave the same as 'bazel clean'\n"
	rememberLine2 = "Remember this choice and skip the prompt in the future"
)

type SelectRunner interface {
	Run() (int, string, error)
}

type PromptRunner interface {
	Run() (string, error)
}

// Clean represents the aspect clean command.
type Clean struct {
	ioutils.Streams
	bzl               bazel.Spawner

	Behavior   SelectRunner
	Workaround PromptRunner
	Remember   PromptRunner
	Prefs      viper.Viper

	Expunge      bool
	ExpungeAsync bool
}

// New creates a Clean command.
func New(
	streams ioutils.Streams,
	bzl bazel.Spawner,
) *Clean {
	return &Clean{
		Streams:           streams,
		bzl:               bzl,
	}
}

func NewDefault() *Clean {
	cleamCmd := New(
		ioutils.DefaultStreams,
		bazel.New())
	cleamCmd.Behavior = &promptui.Select{
		Label: "Clean can have a few behaviors. Which do you want?",
		Items: []string{
			ReclaimOption,
			ReclaimAllOption,
			NonIncrementalOption,
			InvalidateReposOption,
			WorkaroundOption,
		},
	}
	cleamCmd.Workaround = &promptui.Prompt{
		Label:     "Temporarily workaround the bug by deleting the output folder",
		IsConfirm: true,
	}
	cleamCmd.Remember = &promptui.Prompt{
		Label:     rememberLine2,
		IsConfirm: true,
	}
	cleamCmd.Prefs = *viper.GetViper()
	return cleamCmd
}

// Run runs the aspect clean command.
func (cleanCmd *Clean) Run(isInteractiveMode bool) error {
	skip := cleanCmd.Prefs.GetBool(skipPromptKey)
	if isInteractiveMode && !skip {

		_, chosen, err := cleanCmd.Behavior.Run()

		if err != nil {
			return fmt.Errorf("prompt failed: %w", err)
		}

		switch chosen {

		case ReclaimOption:
			// Allow user to opt-out of our fancy "clean" command and just behave like bazel
			fmt.Fprint(cleanCmd.Streams.Stdout, rememberLine1)
			if _, err := cleanCmd.Remember.Run(); err == nil {
				cleanCmd.Prefs.Set(skipPromptKey, "true")
				if err := cleanCmd.Prefs.WriteConfig(); err != nil {
					return fmt.Errorf("failed to update config file: %w", err)
				}
			}
		case ReclaimAllOption:
			fmt.Fprint(cleanCmd.Streams.Stdout, "Sorry, this is not implemented yet: discover all bazel workspaces on the machine\n")
			return nil
		case NonIncrementalOption:
			fmt.Fprint(cleanCmd.Streams.Stdout, outputBaseHint)
			return nil
		case InvalidateReposOption:
			fmt.Fprint(cleanCmd.Streams.Stdout, syncHint)
			return nil
		case WorkaroundOption:
			fmt.Fprint(cleanCmd.Streams.Stdout, fileIssueHint)
			_, err := cleanCmd.Workaround.Run()
			if err != nil {
				return fmt.Errorf("prompt failed: %w", err)
			}
		}
	}

	bazelCmd := []string{"clean"}
	if cleanCmd.Expunge {
		bazelCmd = append(bazelCmd, "--expunge")
	}
	if cleanCmd.ExpungeAsync {
		bazelCmd = append(bazelCmd, "--expunge_async")
	}
	if exitCode, err := cleanCmd.bzl.Spawn(bazelCmd); exitCode != 0 {
		err = &aspecterrors.ExitError{
			Err:      err,
			ExitCode: exitCode,
		}
		return err
	}

	return nil
}
