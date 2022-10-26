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

package client

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"strings"

	hclog "github.com/hashicorp/go-hclog"
	goplugin "github.com/hashicorp/go-plugin"

	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/loader"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha3/config"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha3/plugin"
)

// A Factory class for constructing plugin instances.
type Factory interface {
	New(config loader.AspectPlugin, streams ioutils.Streams) (*PluginInstance, error)
}

func NewFactory() Factory {
	return &clientFactory{}
}

// CustomCommandExecutor requires the Plugin implementations to provide the
// ExecuteCustomCommand method so that the Core can ask over gRPC for a specific command to
// be executed. `cmdName` is the name of the custom command the plugin created.
type CustomCommandExecutor interface {
	ExecuteCustomCommand(cmdName string, ctx context.Context, args []string) error
}

type clientFactory struct {
}

// New calls the goplugin.NewClient with the given config.
func (c *clientFactory) New(aspectplugin loader.AspectPlugin, streams ioutils.Streams) (*PluginInstance, error) {
	logLevel := hclog.LevelFromString(aspectplugin.LogLevel)
	if logLevel == hclog.NoLevel {
		logLevel = hclog.Warn
	}
	pluginLogger := hclog.New(&hclog.LoggerOptions{
		Name:  aspectplugin.Name,
		Level: logLevel,
	})

	if strings.Contains(aspectplugin.From, "/") {
		// Syntax sugar:
		//   from: github.com/org/repo
		// is the same as
		//   from: https://github.com/org/repo/releases/download
		// Example release URL:
		//   https://github.com/aspect-build/aspect-cli-plugin-template/releases/download/v0.1.0/plugin-plugin-linux_amd64
		if strings.HasPrefix(aspectplugin.From, "github.com/") {
			aspectplugin.From = fmt.Sprintf("https://%s/releases/download", aspectplugin.From)
		}
		// Example release URL:
		//   from:          https://static.aspect.build/cli
		//   versioned url: https://static.aspect.build/cli/v0.9.0/plugin-aspect-pro-darwin_amd64
		if strings.HasPrefix(aspectplugin.From, "http://") || strings.HasPrefix(aspectplugin.From, "https://") {
			if len(aspectplugin.Version) < 1 {
				return nil, fmt.Errorf("cannot download plugin '%s': the version field is required", aspectplugin.Name)
			}
			downloaded, err := DownloadPlugin(aspectplugin.From, aspectplugin.Name, aspectplugin.Version)
			if err != nil {
				return nil, err
			}
			aspectplugin.From = downloaded
		} else if _, err := os.Stat(aspectplugin.From); err != nil {
			pluginLogger.Warn(fmt.Sprintf("skipping install for plugin: does not exist at path %s.", aspectplugin.From))
			return nil, nil
		}
	}
	clientConfig := &goplugin.ClientConfig{
		HandshakeConfig:  config.Handshake,
		Plugins:          config.PluginMap,
		Cmd:              exec.Command(aspectplugin.From),
		AllowedProtocols: []goplugin.Protocol{goplugin.ProtocolGRPC},
		SyncStdout:       streams.Stdout,
		SyncStderr:       streams.Stderr,
		Logger:           pluginLogger,
	}

	goclient := goplugin.NewClient(clientConfig)

	rpcClient, err := goclient.Client()
	if err != nil {
		return nil, fmt.Errorf("failed to retrieve plugin client: %w", err)
	}

	rawplugin, err := rpcClient.Dispense(config.DefaultPluginName)
	if err != nil {
		return nil, fmt.Errorf("failed to dispense plugin client: %w", err)
	}

	res := &PluginInstance{
		Plugin:   rawplugin.(plugin.Plugin),
		Provider: goclient,
	}

	if customCommandExecutor, ok := rawplugin.(CustomCommandExecutor); ok {
		res.CustomCommandExecutor = customCommandExecutor
	}

	return res, nil
}

// Provider is an interface for goplugin.Client returned by
// goplugin.NewClient.
type Provider interface {
	Client() (goplugin.ClientProtocol, error)
	Kill()
}

// A PluginInstance consists of the underling Plugin as well
// as any associated objects or metadata.
type PluginInstance struct {
	plugin.Plugin
	Provider
	CustomCommandExecutor
}
