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
	"errors"
	"fmt"
	"os"
	"path"
	"strings"

	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/bazel/workspace"
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

	workspaceConfig, err := LoadWorkspaceConfig()
	if err != nil {
		return err
	}
	if configFlagValues.WorkspaceConfig {
		if err := viper.MergeConfigMap(workspaceConfig.AllSettings()); err != nil {
			return err
		}
	}

	homeConfig, err := LoadHomeConfig()
	if err != nil {
		return err
	}
	if configFlagValues.HomeConfig {
		if err := viper.MergeConfigMap(homeConfig.AllSettings()); err != nil {
			return err
		}
	}

	for _, f := range configFlagValues.UserConfigs {
		if f == "/dev/null" {
			// /dev/null indicates that all further --aspect:config flags will be ignored, which is useful to disable the
			// search for a user config file, such as in release builds.
			break
		}
		userConfig, err := LoadConfigFile(f)
		if err != nil {
			return fmt.Errorf("Failed to load Aspect CLI config file '%s' specified with --aspect:config flag: %w", f, err)
		}
		if err := viper.MergeConfigMap(userConfig.AllSettings()); err != nil {
			return err
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

func WorkspaceConfigFolder() (string, error) {
	cwd, err := os.Getwd()
	if err != nil {
		return "", err
	}
	workspaceRoot, err := workspace.DefaultFinder.Find(cwd)
	if err != nil {
		return "", nil
	}

	return path.Join(workspaceRoot, AspectConfigFolder), nil
}

func WorkspaceConfigFile() (string, error) {
	configFolder, err := WorkspaceConfigFolder()
	if err != nil {
		return "", err
	}

	return path.Join(configFolder, AspectConfigFile), nil
}

func LoadWorkspaceConfig() (*viper.Viper, error) {
	configFile, err := WorkspaceConfigFile()
	if err != nil {
		return nil, err
	}

	return MaybeLoadConfigFile(configFile)
}

// Sets a value in the Aspect CLI $WORKSPACE configuration.
// Returns the path of the config file written to & true if the configuration file is newly created.
func SetInWorkspaceConfig(key string, value interface{}) (string, bool, error) {
	config, err := LoadWorkspaceConfig()
	if err != nil {
		return "", false, err
	}

	configFile, err := WorkspaceConfigFile()
	if err != nil {
		return "", false, err
	}

	configExists, err := exists(configFile)
	if err != nil {
		return "", false, err
	}

	config.Set(key, value)

	if !configExists {
		// Ensure the config directory exists before writing
		if err := os.MkdirAll(AspectConfigFolder, os.ModePerm); err != nil {
			return "", false, err
		}
	}

	return configFile, !configExists, Write(config)
}

func HomeConfigFolder() (string, error) {
	home, err := homedir.Dir()
	if err != nil {
		return "", err
	}

	return path.Join(home, AspectConfigFolder), nil
}

func HomeConfigFile() (string, error) {
	configFolder, err := HomeConfigFolder()
	if err != nil {
		return "", err
	}

	return path.Join(configFolder, AspectConfigFile), nil
}

func LoadHomeConfig() (*viper.Viper, error) {
	configFile, err := HomeConfigFile()
	if err != nil {
		return nil, err
	}

	return MaybeLoadConfigFile(configFile)
}

// Sets a value in the Aspect CLI $HOME configuration.
// Returns the path of the config file written to & true if the configuration file is newly created.
func SetInHomeConfig(key string, value interface{}) (string, bool, error) {
	config, err := LoadHomeConfig()
	if err != nil {
		return "", false, err
	}

	configFile, err := HomeConfigFile()
	if err != nil {
		return "", false, err
	}

	configExists, err := exists(configFile)
	if err != nil {
		return "", false, err
	}

	config.Set(key, value)

	if !configExists {
		// Ensure the config directory exists before writing.
		if err := os.MkdirAll(path.Dir(configFile), os.ModePerm); err != nil {
			return "", false, err
		}
	}

	return configFile, !configExists, Write(config)
}

// Load a config file if it is found
func MaybeLoadConfigFile(f string) (*viper.Viper, error) {
	v := viper.New()
	v.AddConfigPath(path.Dir(f))                                   // the directory to look for the config file in
	v.SetConfigName(strings.TrimSuffix(path.Base(f), path.Ext(f))) // the config file name with extension
	v.SetConfigType(path.Ext(f)[1:])                               // the config file extension without leading dot
	if err := v.ReadInConfig(); err != nil {
		// Ignore "file not found" error for repo config file (it may not exist)
		if _, ok := err.(viper.ConfigFileNotFoundError); !ok {
			return nil, err
		}
	}
	return v, nil
}

// Load a config file and fail if it is not found
func LoadConfigFile(f string) (*viper.Viper, error) {
	v := viper.New()
	v.SetConfigFile(f)
	if err := v.ReadInConfig(); err != nil {
		// Fail is config file specified on the command line is not found
		return nil, err
	}
	return v, nil
}

func exists(name string) (bool, error) {
	_, err := os.Stat(name)
	if err == nil {
		return true, nil
	}
	if errors.Is(err, os.ErrNotExist) {
		return false, nil
	}
	return false, err
}
