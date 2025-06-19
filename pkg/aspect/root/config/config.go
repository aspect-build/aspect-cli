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
	"runtime"
	"strings"

	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel/workspace"
	"github.com/aspect-build/aspect-cli/pkg/plugin/types"
	"github.com/mitchellh/go-homedir"
	"github.com/spf13/pflag"
	"github.com/spf13/viper"
)

type ConfigFlagValues struct {
	UserConfigs     []string
	SystemConfig    bool
	WorkspaceConfig bool
	HomeConfig      bool
}

func AddPlugins(plugins []types.PluginConfig, new []types.PluginConfig) ([]types.PluginConfig, error) {
	for _, n := range new {
		override := false
		for i, p := range plugins {
			if n.Name == p.Name {
				override = true
				plugins[i] = n
				break
			}
		}
		if !override {
			plugins = append(plugins, n)
		}
	}
	return plugins, nil
}

func Load(v *viper.Viper, args []string) error {
	// Load configs in increasing preference. Options in later files can override a value form an
	// earlier file if a conflict arises. Inspired by where Bazel looks for .bazelrc and how this is
	// configured (https://bazel.build/run/bazelrc#bazelrc-file-locations):
	//
	// 1. The system Aspect CLI config file, unless --aspect:nosystem_config is present:
	//
	//    Path: /etc/aspect/cli/config.yaml
	//
	// 2. The workspace Aspect CLI config file, unless --aspect:noworkspace_config is present:
	//
	//    Path: <WORKSPACE>/.aspect/cli/config.yaml
	//
	// 3. The home Aspect CLI config file, unless --aspect:nohome_config is present:
	//
	//    Path: $HOME/.aspect/cli/config.yaml
	//
	// 4. The user-specified Aspect CLI config file, if specified with --aspect:config=<file>
	//
	//    This flag is optional but can also be specified multiple times
	//
	//    /dev/null indicates that all further --aspect:config will be ignored, which is useful to
	//    disable the search for a user rc file, such as in release builds.
	//
	// Viper MergeConfigMap inspired by https://github.com/spf13/viper/issues/181.

	// Parse flags that affect how config files are loaded first. These are a specials flag that must
	// be parsed before we initialize cobra flags since there are some configuration settings such as
	// `version` that need to be checked before doing anything else.
	configFlagValues, err := ParseConfigFlags(args)
	if err != nil {
		return err
	}

	plugins := []types.PluginConfig{}

	if configFlagValues.SystemConfig {
		systemConfig, err := LoadSystemConfig()
		if err != nil {
			return fmt.Errorf("failed to load system config file: %w", err)
		}
		if systemConfig != nil {
			systemPlugins, err := UnmarshalPluginConfig(systemConfig.Get("plugins"))
			if err != nil {
				return fmt.Errorf("failed to load system config file: %w", err)
			}
			plugins, err = AddPlugins(plugins, systemPlugins)
			if err != nil {
				return fmt.Errorf("failed to load system config file: %w", err)
			}
			if err := v.MergeConfigMap(systemConfig.AllSettings()); err != nil {
				return err
			}
		}
	}

	if configFlagValues.WorkspaceConfig {
		workspaceConfig, err := LoadWorkspaceConfig()
		if err != nil {
			// Ignore err if it is a workspace.NotFoundError
			if _, ok := err.(*workspace.NotFoundError); !ok {
				return fmt.Errorf("failed to load workspace config file: %w", err)
			}
		}
		if workspaceConfig != nil {
			workspacePlugins, err := UnmarshalPluginConfig(workspaceConfig.Get("plugins"))
			if err != nil {
				return fmt.Errorf("failed to load workspace config file: %w", err)
			}
			plugins, err = AddPlugins(plugins, workspacePlugins)
			if err != nil {
				return fmt.Errorf("failed to load workspace config file: %w", err)
			}
			if err := v.MergeConfigMap(workspaceConfig.AllSettings()); err != nil {
				return err
			}
		}
	}

	if configFlagValues.HomeConfig {
		homeConfig, err := LoadHomeConfig()
		if err != nil {
			return fmt.Errorf("failed to load home config file: %w", err)
		}
		if homeConfig != nil {
			homePlugins, err := UnmarshalPluginConfig(homeConfig.Get("plugins"))
			if err != nil {
				return fmt.Errorf("failed to load home config file: %w", err)
			}
			plugins, err = AddPlugins(plugins, homePlugins)
			if err != nil {
				return fmt.Errorf("failed to load home config file: %w", err)
			}
			if err := v.MergeConfigMap(homeConfig.AllSettings()); err != nil {
				return err
			}
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
			return fmt.Errorf("failed to load --aspect:config file %q: %w", f, err)
		}
		userPlugins, err := UnmarshalPluginConfig(userConfig.Get("plugins"))
		if err != nil {
			return fmt.Errorf("failed to load --aspect:config file %q: %w", f, err)
		}
		plugins, err = AddPlugins(plugins, userPlugins)
		if err != nil {
			return fmt.Errorf("failed to load --aspect:config file %q: %w", f, err)
		}
		if err := v.MergeConfigMap(userConfig.AllSettings()); err != nil {
			return err
		}
	}

	// Set merged plugins lists
	v.Set("plugins", MarshalPluginConfig(plugins))

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

	systemConfig := flags.RegisterNoableBool(configFlagSet, flags.AspectSystemConfigFlagName, true, "")
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
		SystemConfig:    *systemConfig,
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
		return "", err
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

func SystemConfigFile() string {
	if runtime.GOOS == "darwin" || runtime.GOOS == "linux" {
		return path.Join(AspectSystemConfigFolder, AspectConfigFile)
	}

	return ""
}

func LoadSystemConfig() (*viper.Viper, error) {
	configFile := SystemConfigFile()
	if configFile == "" {
		// no system config file on this OS; this is not an error
		return nil, nil
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
		// Ignore "file not found" error for config file (it may not exist)
		if _, ok := err.(viper.ConfigFileNotFoundError); !ok {
			return nil, fmt.Errorf("failed to load config file %q: %w", f, err)
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

func MarshalPluginConfig(plugins []types.PluginConfig) interface{} {
	l := []interface{}{}
	for _, p := range plugins {
		i := map[string]interface{}{
			"name":                        p.Name,
			"from":                        p.From,
			"multi_threaded_build_events": p.MultiThreadedBuildEvents,
			"disable_bes_events":          p.DisableBESEvents,
		}
		if p.Version != "" {
			i["version"] = p.Version
		}
		if p.LogLevel != "" {
			i["log_level"] = p.LogLevel
		}
		if p.Properties != nil {
			i["properties"] = p.Properties
		}
		l = append(l, i)
	}
	return l
}

func UnmarshalPluginConfig(pluginsConfig interface{}) ([]types.PluginConfig, error) {
	if pluginsConfig == nil {
		return []types.PluginConfig{}, nil
	}

	pluginsList, ok := pluginsConfig.([]interface{})

	if !ok {
		return nil, fmt.Errorf("expected plugins config to be a list")
	}

	plugins := []types.PluginConfig{}

	for i, p := range pluginsList {
		pluginsMap, ok := p.(map[string]interface{})
		if !ok {
			return nil, fmt.Errorf("expected plugins config entry %v to be a map", i)
		}

		name, ok := pluginsMap["name"].(string)
		if !ok {
			return nil, fmt.Errorf("expected plugins config entry %v to have a 'name' attribute", i)
		}

		from, ok := pluginsMap["from"].(string)
		if !ok {
			return nil, fmt.Errorf("expected plugins config entry '%v' to have a 'from' attribute", name)
		}

		version, _ := pluginsMap["version"].(string)
		logLevel, _ := pluginsMap["log_level"].(string)
		multi_threaded_build_events, _ := pluginsMap["multi_threaded_build_events"].(bool)
		disable_bes_events, _ := pluginsMap["disable_bes_events"].(bool)
		properties, _ := pluginsMap["properties"].(map[string]interface{})

		plugins = append(plugins, types.PluginConfig{
			Name:                     name,
			From:                     from,
			Version:                  version,
			LogLevel:                 logLevel,
			MultiThreadedBuildEvents: multi_threaded_build_events,
			DisableBESEvents:         disable_bes_events,
			Properties:               properties,
		})
	}

	return plugins, nil
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
