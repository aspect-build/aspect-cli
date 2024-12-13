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
	"os"
	"reflect"
	"strings"
	"sync"
	"time"

	"github.com/spf13/cobra"
	"golang.org/x/sync/errgroup"
	"google.golang.org/grpc"
	"sigs.k8s.io/yaml"

	"aspect.build/cli/pkg/aspect/root/config"
	"aspect.build/cli/pkg/aspect/root/flags"
	rootFlags "aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/client"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha4/plugin"
	"aspect.build/cli/pkg/plugin/system/bep"
	"aspect.build/cli/pkg/plugin/system/besproxy"
)

// PluginSystem is the interface that defines all the methods for the aspect CLI
// plugin system intended to be used by the Core.
type PluginSystem interface {
	Configure(streams ioutils.Streams, pluginsConfig interface{}) error
	TearDown()
	RegisterCustomCommands(cmd *cobra.Command, bazelStartupArgs []string) error
	BESBackendInterceptor() interceptors.Interceptor
	BESBackendSubscriberInterceptor() interceptors.Interceptor
	BuildHooksInterceptor(streams ioutils.Streams) interceptors.Interceptor
	TestHooksInterceptor(streams ioutils.Streams) interceptors.Interceptor
	RunHooksInterceptor(streams ioutils.Streams) interceptors.Interceptor
}

type pluginSystem struct {
	clientFactory client.Factory
	plugins       *PluginList
	promptRunner  ioutils.PromptRunner
}

// NewPluginSystem instantiates a default internal implementation of the
// PluginSystem interface.
func NewPluginSystem() PluginSystem {
	return &pluginSystem{
		clientFactory: client.NewFactory(),
		plugins:       &PluginList{},
		promptRunner:  ioutils.NewPromptRunner(),
	}
}

// Configure configures the plugin system.
func (ps *pluginSystem) Configure(streams ioutils.Streams, pluginsConfig interface{}) error {
	plugins, err := config.UnmarshalPluginConfig(pluginsConfig)
	if err != nil {
		return fmt.Errorf("failed to configure plugin system: %w", err)
	}

	g := new(errgroup.Group)
	var mutex sync.Mutex

	for _, p := range plugins {
		p := p

		g.Go(func() error {
			aspectplugin, err := ps.clientFactory.New(p, streams)
			if err != nil {
				return err
			}
			if aspectplugin == nil {
				return nil
			}

			properties, err := yaml.Marshal(p.Properties)
			if err != nil {
				return err
			}

			setupConfig := plugin.NewSetupConfig(properties)
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
func (ps *pluginSystem) RegisterCustomCommands(cmd *cobra.Command, bazelStartupArgs []string) error {
	internalCommands := make(map[string]struct{})
	for _, command := range cmd.Commands() {
		cmdName := strings.SplitN(command.Use, " ", 2)[0]
		internalCommands[cmdName] = struct{}{}
	}

	for node := ps.plugins.head; node != nil; node = node.next {
		result, err := node.payload.Plugin.CustomCommands()
		if err != nil {
			return fmt.Errorf("failed to register custom commands: %w", err)
		}

		for _, command := range result {
			cmdName := strings.SplitN(command.Use, " ", 2)[0]
			if _, ok := internalCommands[cmdName]; ok {
				return fmt.Errorf("failed to register custom commands: plugin implements a command with a protected name: %s", command.Use)
			}

			callback := node.payload.CustomCommandExecutor

			cmd.AddCommand(&cobra.Command{
				Use:     command.Use,
				Short:   command.ShortDesc,
				Long:    command.LongDesc,
				GroupID: "plugin",
				RunE: interceptors.Run(
					[]interceptors.Interceptor{},
					func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
						return callback.ExecuteCustomCommand(cmdName, ctx, args, bazelStartupArgs)
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

// BESBackendSubscriberInterceptor always starts a BES backend and injects it into the context.
// Use BESBackendInterceptor to only create the grpc service when there is a known subscriber.
func (ps *pluginSystem) BESBackendSubscriberInterceptor() interceptors.Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next interceptors.RunEContextFn) error {
		return ps.createBesBackend(ctx, cmd, args, next)
	}
}

// BESBackendInterceptor sometimes starts a BES backend and injects it into the context.
// It short-circuits and does nothing in cases where we think there is no subscriber.
// Use BESBackendSubscriberInterceptor if you always know there will be a subscriber.
// It gracefully stops the server after the main command is executed.
func (ps *pluginSystem) BESBackendInterceptor() interceptors.Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next interceptors.RunEContextFn) error {
		// Check if --aspect:force_bes_backend is set. This is primarily used for testing.
		forceBesBackend, err := cmd.Root().Flags().GetBool(rootFlags.AspectForceBesBackendFlagName)
		if err != nil {
			return fmt.Errorf("failed to get value of --aspect:force_bes_backend: %w", err)
		}

		// If there are no plugins configured and --aspect:force_bes_backend is not set then short
		// circuit here since we don't have any need to create a grpc server to consume the build event
		// stream.
		if ps.plugins.head == nil && !forceBesBackend {
			return next(ctx, cmd, args)
		}
		if forceBesBackend {
			fmt.Fprintf(os.Stderr, "Forcing creation of BES backend\n")
		}

		return ps.createBesBackend(ctx, cmd, args, next)
	}
}

func (ps *pluginSystem) createBesBackend(ctx context.Context, cmd *cobra.Command, args []string, next interceptors.RunEContextFn) error {
	// Create the BES backend
	besBackend := bep.NewBESBackend(ctx)
	for node := ps.plugins.head; node != nil; node = node.next {
		besBackend.RegisterSubscriber(node.payload.BEPEventCallback, node.payload.MultiThreaded)
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

	// Check if user has specified --bes_backend
	// https://bazel.build/reference/command-line-reference#flag--bes_backend
	userBesBackend, _ := cmd.Flags().GetString("bes_backend")

	// Configure a BES proxy if `--bes_backend` is set by the user
	if userBesBackend != "" {
		// Check if user has specified any --remote_header values
		// https://bazel.build/reference/command-line-reference#flag--remote_header
		// TODO: parse the --remote_header values from the .bazelrc files in use instead
		userRemoteHeaders := make(map[string]string)
		userRemoteHeader, ok := cmd.Flag("remote_header").Value.(*flags.MultiString)
		if !ok {
			return fmt.Errorf("expected --remote_header flag to be registered with cobra as a MultiString")
		}
		for _, header := range userRemoteHeader.Get() {
			s := strings.Split(header, "=")
			if len(s) != 2 {
				return fmt.Errorf("invalid ---remote_header flag value '%v'; value must be in the form of a 'name=value' assignment", header)
			}
			userRemoteHeaders[s[0]] = s[1]
		}

		besProxy := besproxy.NewBesProxy(userBesBackend, userRemoteHeaders)
		if err := besProxy.Connect(); err != nil {
			fmt.Fprintf(os.Stderr, "Failed to connect to build event stream backend %s: %s", userBesBackend, err.Error())
		} else {
			fmt.Fprintf(os.Stderr, "Forwarding build event stream to %v\n", userBesBackend)
			besBackend.RegisterBesProxy(besProxy)
		}
	}

	// Setup the BES backend grpc server
	if err := besBackend.Setup(opts...); err != nil {
		return fmt.Errorf("failed to run BES backend: %w", err)
	}

	// Start the BES backend
	ctx, cancel := context.WithTimeout(ctx, time.Second)
	defer cancel()
	if err := besBackend.ServeWait(ctx); err != nil {
		return fmt.Errorf("failed to run BES backend: %w", err)
	}
	defer besBackend.GracefulStop()
	ctx = bep.InjectBESBackend(ctx, besBackend)
	return next(ctx, cmd, args)
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
		isInteractiveMode, err := cmd.Root().PersistentFlags().GetBool(rootFlags.AspectInteractiveFlagName)
		if err != nil {
			return fmt.Errorf("failed to run 'aspect %s' command: %w", cmd.CalledAs(), err)
		}

		defer func() {
			hasPluginErrors := false
			for node := ps.plugins.head; node != nil; node = node.next {
				params := []reflect.Value{
					reflect.ValueOf(isInteractiveMode),
					reflect.ValueOf(ps.promptRunner),
				}
				if err := reflect.ValueOf(node.payload).MethodByName(methodName).Call(params)[0].Interface(); err != nil {
					fmt.Fprintf(streams.Stderr, "Error: failed to run 'aspect %s' command: %v\n", cmd.CalledAs(), err)
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
