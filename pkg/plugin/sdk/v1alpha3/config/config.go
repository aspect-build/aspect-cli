/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package config

import (
	goplugin "github.com/hashicorp/go-plugin"

	"aspect.build/cli/pkg/plugin/sdk/v1alpha3/plugin"
)

// DefaultPluginName is the name each aspect plugin must provide.
const DefaultPluginName = "aspectplugin"

// Handshake is the shared handshake config for the v1alpha3 protocol.
var Handshake = goplugin.HandshakeConfig{
	ProtocolVersion:  3,
	MagicCookieKey:   "PLUGIN",
	MagicCookieValue: "ASPECT",
}

// PluginMap represents the plugin interfaces allowed to be implemented by a
// plugin executable.
var PluginMap = map[string]goplugin.Plugin{
	DefaultPluginName: &plugin.GRPCPlugin{},
}

// NewConfigFor returns the default configuration for the passed Plugin
// implementation.
func NewConfigFor(p plugin.Plugin) *goplugin.ServeConfig {
	return &goplugin.ServeConfig{
		HandshakeConfig: Handshake,
		Plugins: map[string]goplugin.Plugin{
			DefaultPluginName: &plugin.GRPCPlugin{Impl: p},
		},
		GRPCServer: goplugin.DefaultGRPCServer,
	}
}
