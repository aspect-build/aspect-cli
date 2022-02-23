/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package system

import (
	"context"
	"errors"
	"fmt"
	"reflect"
	"time"

	yaml "gopkg.in/yaml.v2"

	"github.com/spf13/cobra"

	rootFlags "aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/client"
	"aspect.build/cli/pkg/plugin/loader"
	"aspect.build/cli/pkg/plugin/system/bep"
)

// PluginSystem is the interface that defines all the methods for the aspect CLI
// plugin system intended to be used by the Core.
type PluginSystem interface {
	Configure(streams ioutils.Streams) error
	TearDown()
	RegisterCustomCommands(cmd *cobra.Command) error
	BESBackendInterceptor() interceptors.Interceptor
	BuildHooksInterceptor(streams ioutils.Streams) interceptors.Interceptor
	TestHooksInterceptor(streams ioutils.Streams) interceptors.Interceptor
	RunHooksInterceptor(streams ioutils.Streams) interceptors.Interceptor
}

type pluginSystem struct {
	finder        loader.Finder
	parser        loader.Parser
	clientFactory client.Factory
	plugins       *PluginList
	promptRunner  ioutils.PromptRunner
}

// NewPluginSystem instantiates a default internal implementation of the
// PluginSystem interface.
func NewPluginSystem() PluginSystem {
	return &pluginSystem{
		finder:        loader.NewFinder(),
		parser:        loader.NewParser(),
		clientFactory: client.NewFactory(),
		plugins:       &PluginList{},
		promptRunner:  ioutils.NewPromptRunner(),
	}
}

// Configure configures the plugin system.
func (ps *pluginSystem) Configure(streams ioutils.Streams) error {
	aspectpluginsPath, err := ps.finder.Find()
	if err != nil {
		return fmt.Errorf("failed to configure plugin system: %w", err)
	}
	aspectplugins, err := ps.parser.Parse(aspectpluginsPath)
	if err != nil {
		return fmt.Errorf("failed to configure plugin system: %w", err)
	}

	for _, p := range aspectplugins {
		// TODO(f0rmiga): make this loop concurrent so that all plugins are
		// configured faster.

		aspectplugin, err := ps.clientFactory.New(p, streams)
		if err != nil {
			return fmt.Errorf("failed to configure plugin system: %w", err)
		}

		propertiesBytes, err := yaml.Marshal(p.Properties)
		if err != nil {
			return fmt.Errorf("failed to configure plugin system: %w", err)
		}

		if err := aspectplugin.Setup(propertiesBytes); err != nil {
			return fmt.Errorf("failed to setup plugin: %w", err)
		}

		ps.addPlugin(aspectplugin)
	}

	return nil
}

func (ps *pluginSystem) addPlugin(plugin *client.PluginInstance) {
	ps.plugins.insert(plugin)
}

func (ps *pluginSystem) RegisterCustomCommands(cmd *cobra.Command) error {
	existingCommands := make(map[string]*cobra.Command)

	for _, command := range cmd.Commands() {
		existingCommands[command.Use] = command
	}

	for node := ps.plugins.head; node != nil; node = node.next {
		result, err := node.payload.Plugin.CustomCommands()
		if err != nil {
			return fmt.Errorf("failed to register custom commands: %w", err)
		}

		for _, command := range result {
			if _, ok := existingCommands[command.Use]; ok {
				return fmt.Errorf("failed to register custom commands: plugin implements a command with a protected name: %s", command.Use)
			}

			callback := node.payload.CustomCommandExecutor

			cmd.AddCommand(&cobra.Command{
				Use:   command.Use,
				Short: command.ShortDesc,
				Long:  command.LongDesc,
				RunE: interceptors.Run(
					[]interceptors.Interceptor{
						interceptors.WorkspaceRootInterceptor(),
					},
					func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
						return callback.ExecuteCustomCommand(command.Use, ctx, args)
					},
				),
			})
		}
	}
	return nil
}

// TearDown tears down the plugin system, making all the necessary actions to
// clean up the system.
func (ps *pluginSystem) TearDown() {
	for node := ps.plugins.head; node != nil; node = node.next {
		node.payload.Kill()
	}
}

// BESBackendInterceptorKeyType is a type for the BESBackendInterceptorKey that
// avoids collisions.
type BESBackendInterceptorKeyType bool

// BESBackendInterceptorKeyType is the key for the injected BES backend into
// the context.
const BESBackendInterceptorKey BESBackendInterceptorKeyType = true

// BESBackendInterceptor starts a BES backend and injects it into the context.
// It gracefully stops the  server after the main command is executed.
func (ps *pluginSystem) BESBackendInterceptor() interceptors.Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next interceptors.RunEContextFn) error {
		besBackend := bep.NewBESBackend()
		for node := ps.plugins.head; node != nil; node = node.next {
			besBackend.RegisterSubscriber(node.payload.BEPEventCallback)
		}
		if err := besBackend.Setup(); err != nil {
			return fmt.Errorf("failed to run BES backend: %w", err)
		}
		ctx, cancel := context.WithTimeout(ctx, time.Second)
		defer cancel()
		if err := besBackend.ServeWait(ctx); err != nil {
			return fmt.Errorf("failed to run BES backend: %w", err)
		}
		defer besBackend.GracefulStop()
		ctx = context.WithValue(ctx, BESBackendInterceptorKey, besBackend)
		return next(ctx, cmd, args)
	}
}

// BuildHooksInterceptor returns an interceptor that runs the pre and post-build
// hooks from all plugins.
func (ps *pluginSystem) BuildHooksInterceptor(streams ioutils.Streams) interceptors.Interceptor {
	return ps.commandHooksInterceptor("PostBuildHook", streams)
}

// TestHooksInterceptor returns an interceptor that runs the pre and post-test
// hooks from all plugins.
func (ps *pluginSystem) TestHooksInterceptor(streams ioutils.Streams) interceptors.Interceptor {
	return ps.commandHooksInterceptor("PostTestHook", streams)
}

// TestHooksInterceptor returns an interceptor that runs the pre and post-test
// hooks from all plugins.
func (ps *pluginSystem) RunHooksInterceptor(streams ioutils.Streams) interceptors.Interceptor {
	return ps.commandHooksInterceptor("PostRunHook", streams)
}

func (ps *pluginSystem) commandHooksInterceptor(methodName string, streams ioutils.Streams) interceptors.Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next interceptors.RunEContextFn) (exitErr error) {
		isInteractiveMode, err := cmd.Root().PersistentFlags().GetBool(rootFlags.InteractiveFlagName)
		if err != nil {
			return fmt.Errorf("failed to run 'aspect %s' command: %w", cmd.Use, err)
		}

		defer func() {
			hasPluginErrors := false
			for node := ps.plugins.head; node != nil; node = node.next {
				params := []reflect.Value{
					reflect.ValueOf(isInteractiveMode),
					reflect.ValueOf(ps.promptRunner),
				}
				if err := reflect.ValueOf(node.payload).MethodByName(methodName).Call(params)[0].Interface(); err != nil {
					fmt.Fprintf(streams.Stderr, "Error: failed to run 'aspect %s' command: %v\n", cmd.Use, err)
					hasPluginErrors = true
				}
			}
			if hasPluginErrors {
				var err *aspecterrors.ExitError
				if errors.As(exitErr, &err) {
					err.ExitCode = 1
				}
			}
		}()
		return next(ctx, cmd, args)
	}
}

// PluginList implements a simple linked list for the parsed plugins from the
// plugins file.
type PluginList struct {
	head *PluginNode
	tail *PluginNode
}

func (l *PluginList) insert(p *client.PluginInstance) {
	node := &PluginNode{payload: p}
	if l.head == nil {
		l.head = node
	} else {
		l.tail.next = node
	}
	l.tail = node
}

// PluginNode is a node in the PluginList linked list.
type PluginNode struct {
	next    *PluginNode
	payload *client.PluginInstance
}
