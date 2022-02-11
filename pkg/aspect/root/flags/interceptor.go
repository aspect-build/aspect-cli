/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package flags

import (
	"context"
	"strings"

	"github.com/fatih/color"
	"github.com/mitchellh/go-homedir"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

var (
	faint = color.New(color.Faint)
)

func AddGlobalFlags(cmd *cobra.Command, defaultInteractive bool) {
	cmd.PersistentFlags().String(ConfigFlagName, "", "config file (default is $HOME/.aspect.yaml)")
	cmd.PersistentFlags().Bool(InteractiveFlagName, defaultInteractive, "Interactive mode (e.g. prompts for user input)")
}

func FlagsInterceptor(streams ioutils.Streams) interceptors.Interceptor {
	bzl := bazel.New()
	return flagInterceptor(bzl, streams)
}

func flagInterceptor(bzl bazel.Bazel, streams ioutils.Streams) interceptors.Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next interceptors.RunEContextFn) error {
		if cmd.DisableFlagParsing {
			cmd.DisableFlagParsing = false

			if err := cmd.ParseFlags(args); err != nil {
				return err
			}
		}

		// If user specifies the config file to use then we want to only use that config.
		// If user does not specify a config file to use then we want to load ".aspect" from the
		// $HOME directory and from the root of the repo (if it exists).
		// Adding a second config path using "AddConfigPath" does not work because we dont
		// change the config name using "AddConfigPath". This results in loading the same config
		// file twice. A workaround for this is to have a second viper instance load the repo
		// config and merge them together. Source: https://github.com/spf13/viper/issues/181
		repoViper := viper.New()

		cfgFile, err := cmd.Flags().GetString(ConfigFlagName)
		if err != nil {
			return err
		}

		if cfgFile != "" {
			// Use config file from the flag.
			viper.SetConfigFile(cfgFile)
		} else {
			// Find home directory.
			home, err := homedir.Dir()
			cobra.CheckErr(err)

			// Search for config in home directory with name ".aspect" (without extension).
			viper.AddConfigPath(home)
			viper.SetConfigName(".aspect")

			// Search for config in root of current repo with name ".aspect" (without extension).
			repoViper.AddConfigPath(".")
			repoViper.SetConfigName(".aspect")
			repoViper.AutomaticEnv()
		}

		viper.AutomaticEnv()
		if err := viper.ReadInConfig(); err == nil {
			faint.Fprintln(streams.Stderr, "Using config file:", viper.ConfigFileUsed())
		}

		if err := repoViper.ReadInConfig(); err == nil {
			faint.Fprintln(streams.Stderr, "Using config file:", repoViper.ConfigFileUsed())
		}

		viper.MergeConfigMap(repoViper.AllSettings())

		// Remove "aspect:*" args from the list of args. These should be accessed via cmd.Flags()
		updatedArgs := make([]string, 0)
		for i := 0; i < len(args); i++ {
			if strings.Contains(args[i], "aspect:") {
				continue
			}

			updatedArgs = append(updatedArgs, args[i])
		}

		return next(ctx, cmd, updatedArgs)
	}
}
