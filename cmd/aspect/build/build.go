/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package build

import (
	"github.com/manifoldco/promptui"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	rootFlags "aspect.build/cli/cmd/aspect/root/flags"
	"aspect.build/cli/pkg/aspect/build"
	"aspect.build/cli/pkg/aspect/build/bep"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/hooks"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/pathutils"
	"aspect.build/cli/pkg/plugin/system"
)

// NewDefaultBuildCmd creates a new build cobra command with the default
// dependencies.
func NewDefaultBuildCmd() *cobra.Command {
	return NewBuildCmd(
		ioutils.DefaultStreams,
		bazel.New(),
		bep.NewBESBackend(),
		hooks.New(),
	)
}

// NewBuildCmd creates a new build cobra command.
func NewBuildCmd(
	streams ioutils.Streams,
	bzl bazel.Spawner,
	besBackend bep.BESBackend,
	hooks *hooks.Hooks,
) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "build",
		Short: "Builds the specified targets, using the options.",
		Long: "Invokes bazel build on the specified targets. " +
			"See 'bazel help target-syntax' for details and examples on how to specify targets to build.",
		RunE: func(cmd *cobra.Command, args []string) (exitErr error) {
			return pathutils.InvokeCmdInsideWorkspace(cmd.Use, func() error {
				pluginSystem := system.NewPluginSystem()
				if err := pluginSystem.Configure(streams); err != nil {
					return err
				}
				defer pluginSystem.TearDown()

				for node := pluginSystem.PluginList().Head; node != nil; node = node.Next {
					besBackend.RegisterSubscriber(node.Plugin.BEPEventCallback)
					hooks.RegisterPostBuild(node.Plugin.PostBuildHook)
				}

				isInteractiveMode, err := cmd.Root().PersistentFlags().GetBool(rootFlags.InteractiveFlagName)
				if err != nil {
					return err
				}

				buildCmd := build.New(streams, bzl, besBackend, hooks)
				buildCmd.Behavior = &promptui.Select{
					Label: "What would you like to build?",
					Items: []string{
						build.SpecifiedPackageOption,
						build.CurrentPackageOption,
						build.TargetPatternOption,
					},
				}
				buildCmd.Remember = &promptui.Prompt{
					Label:     build.RememberLine1,
					IsConfirm: true,
				}
				buildCmd.Prefs = *viper.GetViper()
				return buildCmd.Run(cmd.Context(), args, isInteractiveMode)
			})
		},
	}

	return cmd
}
