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
	"fmt"
	"os"
	"strings"

	"github.com/mitchellh/go-homedir"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

// AddGlobalFlags will add aspect specfic flags to all cobra commands.
func AddGlobalFlags(cmd *cobra.Command, defaultInteractive bool) {
	cmd.PersistentFlags().String(ConfigFlagName, "", "config file (default is $HOME/.aspect/cli/config.yaml)")
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

			// Search for config in home directory
			viper.AddConfigPath(home)
			viper.SetConfigName(".aspect/cli/config")
			viper.SetConfigType("yaml")

			// Ensure the config directory exists under the home directory
			os.MkdirAll(fmt.Sprintf("%s/.aspect/cli", home), os.ModePerm)

			// Search for config in root of current repo
			repoViper.AddConfigPath(".")
			repoViper.SetConfigName(".aspect/cli/config")
			repoViper.SetConfigType("yaml")
			repoViper.AutomaticEnv()
		}

		viper.AutomaticEnv()

		// Attempt to read the config files. If we add logging infrastructure, we should log an info
		// when starting to read (viper.ConfigFileUsed(), repoViper.ConfigFileUsed()) and a warning
		// if the file(s) are not found.
		_ = viper.ReadInConfig()
		_ = repoViper.ReadInConfig()

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
