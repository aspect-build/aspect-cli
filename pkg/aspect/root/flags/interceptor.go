/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package flags

import (
	"context"
	"fmt"
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

// AddGlobalFlags will add aspect specfic flags to all cobra commands.
func AddGlobalFlags(cmd *cobra.Command, defaultInteractive bool) {
	cmd.PersistentFlags().String(ConfigFlagName, "", "config file (default is $HOME/.aspect.yaml)")
	cmd.PersistentFlags().Bool(InteractiveFlagName, defaultInteractive, "Interactive mode (e.g. prompts for user input)")
}

// FlagsIntercepor will parse the incoming flags and remove any aspect specific flags or bazel
// startup flags from the list of args.
func FlagsInterceptor(streams ioutils.Streams) interceptors.Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next interceptors.RunEContextFn) error {

		if cmd.DisableFlagParsing {
			cmd.DisableFlagParsing = false

			if err := cmd.ParseFlags(args); err != nil {
				return err
			}
		}

		bzl := bazel.New()
		availableStartupFlags := bzl.AvailableStartupFlags()
		startupFlags := []string{}
		argsWithoutStartupFlags := []string{}

		for _, arg := range args {
			isStartup := false
			for _, availableStartupFlag := range availableStartupFlags {
				if arg == "--"+availableStartupFlag || strings.Contains(arg, "--"+availableStartupFlag+"=") {
					isStartup = true
					break
				}
			}

			if isStartup {
				startupFlags = append(startupFlags, arg)
			} else {
				argsWithoutStartupFlags = append(argsWithoutStartupFlags, arg)
			}
		}

		bzl.SetStartupFlags(startupFlags)
		args = argsWithoutStartupFlags

		// If user specifies the config file to use then we want to only use that config.
		// If user does not specify a config file to use then we want to load ".aspect" from the
		// $HOME directory and from the root of the repo (if it exists).
		// Adding a second config path using "AddConfigPath" does not work because we don't
		// change the config name using "AddConfigPath". This results in loading the same config
		// file twice. A workaround for this is to have a second viper instance load the repo
		// config and merge them together. Source: https://github.com/spf13/viper/issues/181

		cfgFile, err := cmd.Flags().GetString(ConfigFlagName)
		if err != nil {
			return err
		}

		if cfgFile != "" {
			// Use config file from the flag.
			viper.SetConfigFile(cfgFile)
			viper.AutomaticEnv()

			if err := viper.ReadInConfig(); err != nil {
				if _, ok := err.(viper.ConfigFileNotFoundError); ok {
					// this file does not exist where the user set with the flag
					return fmt.Errorf("Failed to read config file from flag not found at: %s", cfgFile)
				} else {
					return fmt.Errorf("Failed when trying to find config file when flag set with %s: %w", cfgFile, err)
				}
			} else {
				faint.Fprintln(streams.Stderr, "Using config file:", viper.ConfigFileUsed())
			}

		} else {
			// used to search in current directory
			viper.AddConfigPath(".")
			viper.SetConfigName(".aspect")
			// this seems to require being explicitly set
			// https://github.com/spf13/viper/issues/109
			// https://github.com/spf13/viper/issues/316
			viper.SetConfigType("yaml")

			// used to search in home directory
			home, err := homedir.Dir()
			cobra.CheckErr(err)

			viper.AutomaticEnv()

			// what do we want to do with error handeling for all of these reads?
			// it should be consistent just not sure what it should be
			// the errors that aren't viper.ConfigFileNotFoundError will still be thrown how should they be handled?

			// search in current directory first
			if err := viper.ReadInConfig(); err != nil {
				if _, ok := err.(viper.ConfigFileNotFoundError); ok {
					// search in home directory next
					viper.AddConfigPath(home)
					if err = viper.ReadInConfig(); err != nil {
						if _, ok := err.(viper.ConfigFileNotFoundError); ok {
							// Config file not found;
							// create new config file if it does not exist
							if err = viper.WriteConfigAs(fmt.Sprintf("%s/.aspect.yaml", home)); err != nil {
								// if you can't write to home directory fail
								return fmt.Errorf("Failed to write Config file in the home directory %s: %w", home, err)
							}
						} else {
							return fmt.Errorf("Failed when reading Config file in the home directory %s: %w", home, err)
						}
					}
				} else {
					return fmt.Errorf("Failed when reading Config file in the current directory: %w", err)
				}
			} else {
				// the first config file is found and now we need to check for a second config file
				// if a second file is found merge them if not continue with just the previous file
				repoViper := viper.New()
				repoViper.AddConfigPath(home)
				repoViper.SetConfigName(".aspect")
				repoViper.SetConfigType("yaml")
				repoViper.AutomaticEnv()

				if err := repoViper.ReadInConfig(); err != nil {
					if _, ok := err.(viper.ConfigFileNotFoundError); !ok {
						return fmt.Errorf("Failed when reading second Config file in the home directory %s: %w", home, err)
					}
				} else {
					// this means there is another file so we have to merge it
					// not sure of the exact order files are currently merged
					// root file should override the home file though
					// would also be good to know if there is ever a user aspect file
					viper.MergeConfigMap(repoViper.AllSettings())
				}
			}
		}

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
