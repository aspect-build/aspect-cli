/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package build

import (
	"os"

	"github.com/manifoldco/promptui"
	"github.com/mattn/go-isatty"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	"aspect.build/cli/pkg/aspect/build"
	"aspect.build/cli/pkg/aspect/build/bep"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/hooks"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugins/fix_visibility"
)

// NewDefaultBuildCmd creates a new build cobra command with the default
// dependencies.
func NewDefaultBuildCmd() *cobra.Command {
	isInteractive := isatty.IsTerminal(os.Stdout.Fd()) || isatty.IsCygwinTerminal(os.Stdout.Fd())
	return NewBuildCmd(
		ioutils.DefaultStreams,
		bazel.New(),
		isInteractive,
		bep.NewBESBackend(),
		hooks.New(),
	)
}

// NewBuildCmd creates a new build cobra command.
func NewBuildCmd(
	streams ioutils.Streams,
	bzl bazel.Spawner,
	isInteractive bool,
	besBackend bep.BESBackend,
	hooks *hooks.Hooks,
) *cobra.Command {
	// TODO(f0rmiga): this should also be part of the plugin design, as
	// registering BEP event subscribers should not be hardcoded here.
	var fixVisibilityPlugin build.Plugin = fix_visibility.NewDefaultPlugin()
	besBackend.RegisterSubscriber(fixVisibilityPlugin.BEPEventCallback)
	hooks.RegisterPostBuild(fixVisibilityPlugin.PostBuildHook)

	b := build.New(
		streams,
		bzl,
		isInteractive,
		besBackend,
		hooks,
	)
	b.Behavior = &promptui.Select{
		Label: "What would you like to build?",
		Items: []string{
			build.SpecifiedFolderOption,
			build.CurrentFolderOption,
			build.TargetPatternOption,
		},
	}
	b.Remember = &promptui.Prompt{
		Label:     build.RememberLine1,
		IsConfirm: true,
	}
	b.Prefs = *viper.GetViper()

	cmd := &cobra.Command{
		Use:   "build",
		Short: "Builds the specified targets, using the options.",
		Long: "Invokes bazel build on the specified targets. " +
			"See 'bazel help target-syntax' for details and examples on how to specify targets to build.",
		RunE: func(cmd *cobra.Command, args []string) (exitErr error) {
			return b.Run(cmd.Context(), cmd, args)
		},
	}

	return cmd
}
