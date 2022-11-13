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

package config

import (
	"fmt"
	"os"
	"path"

	"aspect.build/cli/pkg/aspect/root/flags"
	"github.com/mitchellh/go-homedir"
	"github.com/spf13/pflag"
	"github.com/spf13/viper"
)

type ConfigFlagValues struct {
	UserConfigs     []string
	WorkspaceConfig bool
	HomeConfig      bool
}

func Load(args []string) error {
	// Load configs in increasing preference. Options in later files can override a value form an
	// earlier file if a conflict arises. Inspired by where Bazel looks for .bazelrc and how this is
	// configured: https://bazel.build/run/bazelrc. Viper merge pattern from
	// https://github.com/spf13/viper/issues/181.

	// Parse flags that affect how config files are loaded first. These are a specials flag that must
	// be parsed before we initialize cobra flags since there are some configuration settings such as
	// `version` that need to be checked before doing anything else.
	configFlagValues, err := ParseConfigFlags(args)
	if err != nil {
		return err
	}

	if err := loadWorkspaceConfig(configFlagValues); err != nil {
		return err
	}

	if err := loadHomeConfig(configFlagValues); err != nil {
		return err
	}

	for _, f := range configFlagValues.UserConfigs {
		if f == "/dev/null" {
			// /dev/null indicates that all further --aspect:config flags will be ignored, which is useful to disable the
			// search for a user config file, such as in release builds.
			break
		}
		if err := loadConfigFile(f); err != nil {
			return fmt.Errorf("Failed to load Aspect CLI config file '%s' specified with --aspect:config flag: %w", f, err)
		}
	}

	return nil
}

func ParseConfigFlags(args []string) (*ConfigFlagValues, error) {
	configFlagSet := pflag.NewFlagSet(args[0], pflag.ContinueOnError)

	// Ignore unknown flags
	configFlagSet.ParseErrorsWhitelist.UnknownFlags = true

	// Silence usage output
	configFlagSet.Usage = func() {}

	var userConfigs = flags.MultiString{}
	configFlagSet.Var(&userConfigs, flags.AspectConfigFlagName, "")

	workspaceConfig := flags.RegisterNoableBool(configFlagSet, flags.AspectWorkspaceConfigFlagName, true, "")
	homeConfig := flags.RegisterNoableBool(configFlagSet, flags.AspectHomeConfigFlagName, true, "")

	if err := configFlagSet.Parse(args[1:]); err != nil {
		// Ignore the special help requested pflag error case
		if err != pflag.ErrHelp {
			return nil, err
		}
	}

	return &ConfigFlagValues{
		UserConfigs:     userConfigs.Get(),
		WorkspaceConfig: *workspaceConfig,
		HomeConfig:      *homeConfig,
	}, nil
}

func loadWorkspaceConfig(configFlagValues *ConfigFlagValues) error {
	if configFlagValues.WorkspaceConfig {
		// Search for config in root of current repo
		return maybeLoadConfigFile(fmt.Sprintf("%s/%s", AspectConfigFolder, AspectConfigFile))
	}
	return nil
}

func loadHomeConfig(configFlagValues *ConfigFlagValues) error {
	home, err := homedir.Dir()
	if err != nil {
		return err
	}

	homeConfigFolder := fmt.Sprintf("%s/%s", home, AspectConfigFolder)

	// Ensure the config directory exists under the home directory.
	// This is so that viper can create the user home config if desired.
	os.MkdirAll(homeConfigFolder, os.ModePerm)

	if configFlagValues.HomeConfig {
		// Search for config in the user home directory
		return maybeLoadConfigFile(fmt.Sprintf("%s/%s", homeConfigFolder, AspectConfigFile))
	}
	return nil
}

func maybeLoadConfigFile(f string) error {
	v := viper.New()
	v.AddConfigPath(path.Dir(f))
	v.SetConfigName(path.Base(f))
	if err := v.ReadInConfig(); err != nil {
		// Ignore "file not found" error for repo config file (it may not exist)
		if _, ok := err.(viper.ConfigFileNotFoundError); !ok {
			return err
		}
	}
	if err := viper.MergeConfigMap(v.AllSettings()); err != nil {
		return err
	}
	return nil
}

func loadConfigFile(f string) error {
	v := viper.New()
	v.SetConfigFile(f)
	if err := v.ReadInConfig(); err != nil {
		// Fail is config file specified on the command line is not found
		return err
	}
	if err := viper.MergeConfigMap(v.AllSettings()); err != nil {
		return err
	}
	return nil
}
