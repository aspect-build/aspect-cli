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

package plugin

import (
	"context"
	"fmt"
	"strings"

	buildeventstream "github.com/aspect-build/aspect-cli/bazel/buildeventstream"
	"github.com/aspect-build/aspect-cli/pkg/ioutils/prompt"
	"github.com/aspect-build/aspect-cli/pkg/plugin/sdk/v1alpha4/proto"
)

// Plugin determines how an aspect Plugin should be implemented.
type Plugin interface {
	BEPEventCallback(event *buildeventstream.BuildEvent, sn int64) error
	CustomCommands() ([]*Command, error)
	PostBuildHook(
		isInteractiveMode bool,
		promptRunner prompt.PromptRunner,
	) error
	PostTestHook(
		isInteractiveMode bool,
		promptRunner prompt.PromptRunner,
	) error
	PostRunHook(
		isInteractiveMode bool,
		promptRunner prompt.PromptRunner,
	) error
	Setup(config *SetupConfig) error
}

// SetupConfig represents a plugin configuration parsed from the aspectplugins
// file.
type SetupConfig struct {
	File       *AspectPluginFile
	Properties []byte
}

// NewSetupConfig creates a new SetupConfig.
func NewSetupConfig(
	properties []byte,
) *SetupConfig {
	return &SetupConfig{
		File:       &AspectPluginFile{Path: ""},
		Properties: properties,
	}
}

// AspectPluginFile contains metadata for the aspectplugins file relevant for
// a plugin.
type AspectPluginFile struct {
	Path string
}

// NewAspectPluginFile creates a new AspectPluginFile.
func NewAspectPluginFile(path string) *AspectPluginFile {
	return &AspectPluginFile{
		Path: path,
	}
}

// Base satisfies the Plugin interface. For plugins that only implement a subset
// of the Plugin interface, using this as a base will give the advantage of not
// needing to implement the empty methods.
type Base struct{}

var _ Plugin = (*Base)(nil)

// Setup satisfies Plugin.Setup.
func (*Base) Setup(*SetupConfig) error {
	return nil
}

// BEPEventCallback satisfies Plugin.BEPEventCallback.
func (*Base) BEPEventCallback(*buildeventstream.BuildEvent, int64) error {
	return nil
}

// CustomCommands satisfies Plugin.BEPEventCallback.
func (*Base) CustomCommands() ([]*Command, error) {
	return nil, nil
}

// PostBuildHook satisfies Plugin.PostBuildHook.
func (*Base) PostBuildHook(bool, prompt.PromptRunner) error {
	return nil
}

// PostTestHook satisfies Plugin.PostTestHook.
func (*Base) PostTestHook(bool, prompt.PromptRunner) error {
	return nil
}

// PostRunHook satisfies Plugin.PostRunHook.
func (*Base) PostRunHook(bool, prompt.PromptRunner) error {
	return nil
}

// CustomCommandFn defines the parameters of that the Run functions will be called with.
type CustomCommandFn (func(ctx context.Context, args []string, bazelStartupArgs []string) error)

// Command defines the information needed to create a custom command that will be callable when
// running the CLI.
type Command struct {
	*proto.Command
	Run CustomCommandFn
}

// NewCommand is a wrapper around Command. Designed to be used as a cleaner way to make a Command
// given Command's nested proto
func NewCommand(
	use string,
	shortDesc string,
	longDesc string,
	run CustomCommandFn,
) *Command {
	return &Command{
		Command: &proto.Command{
			Use:       use,
			ShortDesc: shortDesc,
			LongDesc:  longDesc,
		},
		Run: run,
	}
}

// CommandManager is internal to the SDK and is used to manage custom commands that
// are provided by plugins.
type CommandManager interface {
	Save(commands []*Command) error
	Execute(command string, ctx context.Context, args []string, bazelStartupArgs []string) error
}

// PluginCommandManager is internal to the SDK and is used to manage custom commands that
// are provided by plugins.
type PluginCommandManager struct {
	commands map[string]CustomCommandFn
}

// Save satisfies CommandManager.
func (cm *PluginCommandManager) Save(commands []*Command) error {
	for _, cmd := range commands {
		cmdName := strings.SplitN(cmd.Use, " ", 2)[0]
		if _, exists := cm.commands[cmdName]; exists {
			return fmt.Errorf("command %q is declared more than once by plugin", cmdName)
		}
		cm.commands[cmdName] = cmd.Run
	}

	return nil
}

// Execute satisfies CommandManager.
func (cm *PluginCommandManager) Execute(command string, ctx context.Context, args []string, bazelStartupArgs []string) error {
	return cm.commands[command](ctx, args, bazelStartupArgs)
}

var _ CommandManager = (*PluginCommandManager)(nil)
