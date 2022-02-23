/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package plugin

import (
	"context"
	"fmt"

	buildeventstream "aspect.build/cli/bazel/buildeventstream/proto"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha2/proto"
)

// Plugin determines how an aspect Plugin should be implemented.
type Plugin interface {
	BEPEventCallback(event *buildeventstream.BuildEvent) error
	CustomCommands() ([]*Command, error)
	PostBuildHook(
		isInteractiveMode bool,
		promptRunner ioutils.PromptRunner,
	) error
	PostTestHook(
		isInteractiveMode bool,
		promptRunner ioutils.PromptRunner,
	) error
	PostRunHook(
		isInteractiveMode bool,
		promptRunner ioutils.PromptRunner,
	) error
	Setup(properties []byte) error
}

// Base satisfies the Plugin interface. For plugins that only implement a subset
// of the Plugin interface, using this as a base will give the advantage of not
// needing to implement the empty methods.
type Base struct{}

var _ Plugin = (*Base)(nil)

// Setup satisfies Plugin.Setup.
func (*Base) Setup([]byte) error {
	return nil
}

// BEPEventCallback satisfies Plugin.BEPEventCallback.
func (*Base) BEPEventCallback(*buildeventstream.BuildEvent) error {
	return nil
}

// BEPEventCallback satisfies Plugin.BEPEventCallback.
func (*Base) CustomCommands() ([]*Command, error) {
	return nil, nil
}

// PostBuildHook satisfies Plugin.PostBuildHook.
func (*Base) PostBuildHook(bool, ioutils.PromptRunner) error {
	return nil
}

// PostTestHook satisfies Plugin.PostTestHook.
func (*Base) PostTestHook(bool, ioutils.PromptRunner) error {
	return nil
}

// PostRunHook satisfies Plugin.PostRunHook.
func (*Base) PostRunHook(bool, ioutils.PromptRunner) error {
	return nil
}

type CustomCommandFn (func(ctx context.Context, args []string, bzl bazel.Bazel) error)

type Command struct {
	*proto.Command
	Run CustomCommandFn
}

func NewCommand(
	use string,
	shortDesc string,
	longDesc string,
	run func(ctx context.Context, args []string, bzl bazel.Bazel) error,
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

type CommandManager interface {
	Save(commands []*Command) error
	Execute(command string, ctx context.Context, args []string) error // Not sure about this signature, haven't read the whole code yet.
}

type PluginCommandManager struct {
	commands map[string]CustomCommandFn
}

func (cm *PluginCommandManager) Save(commands []*Command) error {
	for _, cmd := range commands {
		if _, exists := cm.commands[cmd.Use]; exists {
			return fmt.Errorf("command '%s' is declared more than once by plugin", cmd.Use)
		}
		cm.commands[cmd.Use] = cmd.Run
	}

	return nil
}

func (cm *PluginCommandManager) Execute(command string, ctx context.Context, args []string) error {
	workspaceRoot := ctx.Value(interceptors.WorkspaceRootKey).(string)
	bzl := bazel.New()
	bzl.SetWorkspaceRoot(workspaceRoot)

	return cm.commands[command](ctx, args, bzl)
}

var _ CommandManager = (*PluginCommandManager)(nil)
