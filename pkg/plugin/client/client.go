/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package client

import (
	"context"
	"fmt"
	"os/exec"

	hclog "github.com/hashicorp/go-hclog"
	goplugin "github.com/hashicorp/go-plugin"

	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/loader"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha2/config"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha2/plugin"
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

type clientFactory struct{}

// New calls the goplugin.NewClient with the given config.
func (*clientFactory) New(aspectplugin loader.AspectPlugin, streams ioutils.Streams) (*PluginInstance, error) {
	logLevel := hclog.LevelFromString(aspectplugin.LogLevel)
	if logLevel == hclog.NoLevel {
		logLevel = hclog.Error
	}
	pluginLogger := hclog.New(&hclog.LoggerOptions{
		Name:  aspectplugin.Name,
		Level: logLevel,
	})

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
		return nil, fmt.Errorf("failed to configure plugin client: %w", err)
	}

	rawplugin, err := rpcClient.Dispense(config.DefaultPluginName)
	if err != nil {
		return nil, fmt.Errorf("failed to configure plugin client: %w", err)
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
	CustomCommandExecutor CustomCommandExecutor
}
