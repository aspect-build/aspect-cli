/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package system

import (
	"context"
	"errors"
	"fmt"
	"os/exec"
	"reflect"
	"time"

	hclog "github.com/hashicorp/go-hclog"
	goplugin "github.com/hashicorp/go-plugin"
	"github.com/spf13/cobra"

	rootFlags "aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha2/config"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha2/plugin"
	"aspect.build/cli/pkg/plugin/system/bep"
)

// PluginSystem is the interface that defines all the methods for the aspect CLI
// plugin system intended to be used by the Core.
type PluginSystem interface {
	ConfigurePluginSystem(streams ioutils.Streams) error
	TearDown()
	BESBackendInterceptor() interceptors.Interceptor
	BuildHooksInterceptor(streams ioutils.Streams) interceptors.Interceptor
	ConfigurePlugins() *aspecterrors.ErrorList
	TestHooksInterceptor(streams ioutils.Streams) interceptors.Interceptor
	RunHooksInterceptor(streams ioutils.Streams) interceptors.Interceptor
}

type pluginSystem struct {
	finder        Finder
	parser        Parser
	clientFactory ClientFactory
	clients       []ClientProvider
	plugins       *PluginList
	promptRunner  ioutils.PromptRunner
}

// NewPluginSystem instantiates a default internal implementation of the
// PluginSystem interface.
func NewPluginSystem() PluginSystem {
	return &pluginSystem{
		finder:        NewFinder(),
		parser:        NewParser(),
		clientFactory: &clientFactory{},
		plugins:       &PluginList{},
		promptRunner:  ioutils.NewPromptRunner(),
	}
}

// ConfigurePluginSystem configures the plugin system.
func (ps *pluginSystem) ConfigurePluginSystem(streams ioutils.Streams) error {
	aspectpluginsPath, err := ps.finder.Find()
	if err != nil {
		return fmt.Errorf("failed to configure plugin system: %w", err)
	}
	aspectplugins, err := ps.parser.Parse(aspectpluginsPath)
	if err != nil {
		return fmt.Errorf("failed to configure plugin system: %w", err)
	}

	ps.clients = make([]ClientProvider, 0, len(aspectplugins))
	for _, aspectplugin := range aspectplugins {
		logLevel := hclog.LevelFromString(aspectplugin.LogLevel)
		if logLevel == hclog.NoLevel {
			logLevel = hclog.Error
		}
		pluginLogger := hclog.New(&hclog.LoggerOptions{
			Name:  aspectplugin.Name,
			Level: logLevel,
		})
		// TODO(f0rmiga): make this loop concurrent so that all plugins are
		// configured faster.
		clientConfig := &goplugin.ClientConfig{
			HandshakeConfig:  config.Handshake,
			Plugins:          config.PluginMap,
			Cmd:              exec.Command(aspectplugin.From),
			AllowedProtocols: []goplugin.Protocol{goplugin.ProtocolGRPC},
			SyncStdout:       streams.Stdout,
			SyncStderr:       streams.Stderr,
			Logger:           pluginLogger,
		}
		client := ps.clientFactory.New(clientConfig)
		ps.clients = append(ps.clients, client)

		rpcClient, err := client.Client()
		if err != nil {
			return fmt.Errorf("failed to configure plugin system: %w", err)
		}

		rawplugin, err := rpcClient.Dispense(config.DefaultPluginName)
		if err != nil {
			return fmt.Errorf("failed to configure plugin system: %w", err)
		}

		node := &PluginNode{
			plugin:     rawplugin.(plugin.Plugin),
			properties: aspectplugin.propertiesBytes,
		}
		ps.plugins.insert(node)
	}

	return nil
}

// TearDown tears down the plugin system, making all the necessary actions to
// clean up the system.
func (ps *pluginSystem) TearDown() {
	for _, client := range ps.clients {
		client.Kill()
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
			besBackend.RegisterSubscriber(node.plugin.BEPEventCallback)
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

// ConfigurePlugins executes the Setup for all configured plugins and provides
// those plugins with their configured properties
func (ps *pluginSystem) ConfigurePlugins() *aspecterrors.ErrorList {
	errors := &aspecterrors.ErrorList{}
	for node := ps.plugins.head; node != nil; node = node.next {
		if err := node.plugin.Setup(node.properties); err != nil {
			errors.Insert(err)
		}
	}
	return errors
}

func (ps *pluginSystem) commandHooksInterceptor(methodName string, streams ioutils.Streams) interceptors.Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next interceptors.RunEContextFn) (exitErr error) {
		isInteractiveMode, err := cmd.Root().PersistentFlags().GetBool(rootFlags.InteractiveFlagName)
		if err != nil {
			return fmt.Errorf("failed to run 'aspect %s' command: %w", cmd.Use, err)
		}

		// TODO(f0rmiga): test this hook.
		defer func() {
			hasErrors := false
			for node := ps.plugins.head; node != nil; node = node.next {
				params := []reflect.Value{
					reflect.ValueOf(isInteractiveMode),
					reflect.ValueOf(ps.promptRunner),
				}
				if err := reflect.ValueOf(node.plugin).MethodByName(methodName).Call(params)[0].Interface(); err != nil {
					fmt.Fprintf(streams.Stderr, "Error: failed to run 'aspect %s' command: %v\n", cmd.Use, err)
					hasErrors = true
				}
			}
			if hasErrors {
				var err *aspecterrors.ExitError
				if errors.As(exitErr, &err) {
					err.ExitCode = 1
				}
			}
		}()
		return next(ctx, cmd, args)
	}
}

// ClientFactory hides the call to goplugin.NewClient.
type ClientFactory interface {
	New(*goplugin.ClientConfig) ClientProvider
}

type clientFactory struct{}

// New calls the goplugin.NewClient with the given config.
func (*clientFactory) New(config *goplugin.ClientConfig) ClientProvider {
	return goplugin.NewClient(config)
}

// ClientProvider is an interface for goplugin.Client returned by
// goplugin.NewClient.
type ClientProvider interface {
	Client() (goplugin.ClientProtocol, error)
	Kill()
}

// PluginList implements a simple linked list for the parsed plugins from the
// plugins file.
type PluginList struct {
	head *PluginNode
	tail *PluginNode
}

func (l *PluginList) insert(node *PluginNode) {
	if l.head == nil {
		l.head = node
	} else {
		l.tail.next = node
	}
	l.tail = node
}

// PluginNode is a node in the PluginList linked list.
type PluginNode struct {
	next       *PluginNode
	plugin     plugin.Plugin
	properties []byte
}
