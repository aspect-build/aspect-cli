/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
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
