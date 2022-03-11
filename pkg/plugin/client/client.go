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
	"strings"

	hclog "github.com/hashicorp/go-hclog"
	goplugin "github.com/hashicorp/go-plugin"

	"aspect.build/cli/pkg/bazel"
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
	return &clientFactory{
		bzl: bazel.New(),
	}
}

// CustomCommandExecutor requires the Plugin implementations to provide the
// ExecuteCustomCommand method so that the Core can ask over gRPC for a specific command to
// be executed. `cmdName` is the name of the custom command the plugin created.
type CustomCommandExecutor interface {
	ExecuteCustomCommand(cmdName string, ctx context.Context, args []string) error
}

type clientFactory struct {
	bzl bazel.Bazel
}

// buildPlugin asks bazel to build the target and returns the path to the resulting binary.
func (c *clientFactory) buildPlugin(target string) (string, error) {
	queryOutput, err := c.bzl.AQuery(target)
	if err != nil {
		return "", err
	}
	outs := bazel.ParseOutputs(queryOutput)

	var pluginPath string
	for _, a := range outs {
		// TODO: don't hard-code GoLink, plugins could be written in other languages
		// https://github.com/aspect-build/aspect-cli/issues/179
		if a.Mnemonic == "GoLink" {
			pluginPath = a.Path
			break
		}
	}
	if pluginPath == "" {
		return "", fmt.Errorf("failed to build plugin %q with Bazel: no output file from a GoLink action was found", target)
	}

	// WARNING: be careful to use flags for this build matching the .bazelrc
	// to avoid busting the analysis cache. We want to pretend to be a typical
	// build the developer or CI would be performing.
	// This is important only in the setup we don't recommend, where normal users
	// are building the plugin from source instead of a pre-built binary.
	if _, err := c.bzl.Spawn([]string{"build", target}); err != nil {
		return "", fmt.Errorf("failed to build plugin %q with Bazel: %w", target, err)
	}

	return pluginPath, nil
}

// New calls the goplugin.NewClient with the given config.
func (c *clientFactory) New(aspectplugin loader.AspectPlugin, streams ioutils.Streams) (*PluginInstance, error) {
	logLevel := hclog.LevelFromString(aspectplugin.LogLevel)
	if logLevel == hclog.NoLevel {
		logLevel = hclog.Error
	}
	pluginLogger := hclog.New(&hclog.LoggerOptions{
		Name:  aspectplugin.Name,
		Level: logLevel,
	})

	if strings.HasPrefix(aspectplugin.From, "//") {
		if built, err := c.buildPlugin(aspectplugin.From); err != nil {
			return nil, err
		} else {
			aspectplugin.From = built
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
	CustomCommandExecutor
}
