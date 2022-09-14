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
	goplugin "github.com/hashicorp/go-plugin"

	"aspect.build/cli/pkg/plugin/sdk/v1alpha2/plugin"
)

// DefaultPluginName is the name each aspect plugin must provide.
const DefaultPluginName = "aspectplugin"

// Handshake is the shared handshake config for the v1alpha2 protocol.
var Handshake = goplugin.HandshakeConfig{
	ProtocolVersion:  2,
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
