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

package system

import (
	"context"
	"errors"
	"fmt"
	"math"
	"reflect"
	"sync"
	"time"

	"github.com/spf13/cobra"
	"golang.org/x/sync/errgroup"
	"google.golang.org/grpc"
	yaml "gopkg.in/yaml.v2"

	rootFlags "aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/client"
	"aspect.build/cli/pkg/plugin/loader"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha3/plugin"
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
	finder         loader.Finder
	parser         loader.Parser
	clientFactory  client.Factory
	plugins        *PluginList
	promptRunner   ioutils.PromptRunner
	defaultPlugins []loader.AspectPlugin
}

// NewDefaultPluginSystem instantiates a default internal implementation of the
// PluginSystem interface.
func NewDefaultPluginSystem() PluginSystem {
	return &pluginSystem{
		finder:         loader.NewFinder(),
		parser:         loader.NewParser(),
		clientFactory:  client.NewFactory(),
		plugins:        &PluginList{},
		promptRunner:   ioutils.NewPromptRunner(),
		defaultPlugins: []loader.AspectPlugin{},
	}
}

// NewPluginSystem instantiates an implementation of the PluginSystem interface
// that allow default plugins to be specified.
func NewPluginSystem(defaultPlugins []loader.AspectPlugin) PluginSystem {
	return &pluginSystem{
		finder:         loader.NewFinder(),
		parser:         loader.NewParser(),
		clientFactory:  client.NewFactory(),
		plugins:        &PluginList{},
		promptRunner:   ioutils.NewPromptRunner(),
		defaultPlugins: defaultPlugins,
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

	// Put the default plugins in a map for later use
	defaultPluginsMap := map[string]loader.AspectPlugin{}
	for _, p := range ps.defaultPlugins {
		defaultPluginsMap[p.Name] = p
	}

	g := new(errgroup.Group)
	var mutex sync.Mutex

	// TODO: support merging plugin configurations from defaultPlugins, workspace
	// plugins.yaml & user plugins.yaml

	for _, p := range aspectplugins {
		p := p

		defaultPlugin, ok := defaultPluginsMap[p.Name]
		if ok {
			if p.From == "" && p.Version == "" {
				p.From = defaultPlugin.From
				p.Version = defaultPlugin.Version
			}
		}

		g.Go(func() error {
			aspectplugin, err := ps.clientFactory.New(p, streams)
			if err != nil {
				return err
			}

			properties, err := yaml.Marshal(p.Properties)
			if err != nil {
				return err
			}

			aspectPluginFile := plugin.NewAspectPluginFile(aspectpluginsPath)
			setupConfig := plugin.NewSetupConfig(aspectPluginFile, properties)
			if err := aspectplugin.Setup(setupConfig); err != nil {
				return err
			}

			mutex.Lock()
			ps.plugins.insert(aspectplugin)
			mutex.Unlock()
			return nil
		})
	}

	if err := g.Wait(); err != nil {
		return fmt.Errorf("failed to configure plugin system: %w", err)
	}

	return nil
}

// RegisterCustomCommands processes custom commands provided by plugins and adds
// them as commands to the core whilst setting up callbacks for the those commands.
func (ps *pluginSystem) RegisterCustomCommands(cmd *cobra.Command) error {
	internalCommands := make(map[string]struct{})

	for _, command := range cmd.Commands() {
		internalCommands[command.Use] = struct{}{}
	}

	for node := ps.plugins.head; node != nil; node = node.next {
		result, err := node.payload.Plugin.CustomCommands()
		if err != nil {
			return fmt.Errorf("failed to register custom commands: %w", err)
		}

		for _, command := range result {
			if _, ok := internalCommands[command.Use]; ok {
				return fmt.Errorf("failed to register custom commands: plugin implements a command with a protected name: %s", command.Use)
			}

			callback := node.payload.CustomCommandExecutor

			cmd.AddCommand(&cobra.Command{
				Use:   command.Use,
				Short: command.ShortDesc,
				Long:  command.LongDesc,
				RunE: interceptors.Run(
					[]interceptors.Interceptor{},
					func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
						return callback.ExecuteCustomCommand(cmd.Use, ctx, args)
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

// BESBackendInterceptor starts a BES backend and injects it into the context.
// It gracefully stops the  server after the main command is executed.
func (ps *pluginSystem) BESBackendInterceptor() interceptors.Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next interceptors.RunEContextFn) error {
		besBackend := bep.NewBESBackend()
		for node := ps.plugins.head; node != nil; node = node.next {
			besBackend.RegisterSubscriber(node.payload.BEPEventCallback)
		}
		opts := []grpc.ServerOption{
			// Bazel doesn't seem to set a maximum send message size, therefore
			// we match the default send message for Go, which should be enough
			// for all messages sent by Bazel (roughly 2.14GB).
			grpc.MaxRecvMsgSize(math.MaxInt32),
			// Here we are just being explicit with the default value since we
			// also set the receive message size.
			grpc.MaxSendMsgSize(math.MaxInt32),
		}
		if err := besBackend.Setup(opts...); err != nil {
			return fmt.Errorf("failed to run BES backend: %w", err)
		}
		ctx, cancel := context.WithTimeout(ctx, time.Second)
		defer cancel()
		if err := besBackend.ServeWait(ctx); err != nil {
			return fmt.Errorf("failed to run BES backend: %w", err)
		}
		defer besBackend.GracefulStop()
		ctx = bep.InjectBESBackend(ctx, besBackend)
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

// RunHooksInterceptor returns an interceptor that runs the pre and post-run
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
