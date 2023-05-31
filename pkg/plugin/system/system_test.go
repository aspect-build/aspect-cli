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
	"strings"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
	"github.com/spf13/cobra"
	"sigs.k8s.io/yaml"

	rootFlags "aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/client"
	client_mock "aspect.build/cli/pkg/plugin/client/mock"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha4/plugin"
	plugin_mock "aspect.build/cli/pkg/plugin/sdk/v1alpha4/plugin/mock"
	"aspect.build/cli/pkg/plugin/types"
)

func createInterceptorCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use: "TestCommand",
	}

	// Required flags for interceptor hooks
	cmd.PersistentFlags().Bool(rootFlags.AspectInteractiveFlagName, false, "")

	return cmd
}

func TestPluginSystemInterceptors(t *testing.T) {
	t.Run("executes hooks in reverse order of interceptors", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		// Setup
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout, Stderr: &stdout}
		ctx := context.Background()
		cmd := createInterceptorCommand()

		ps := NewPluginSystem().(*pluginSystem)
		plugin := plugin_mock.NewMockPlugin(ctrl)
		ps.plugins.insert(&client.PluginInstance{
			Plugin:   plugin,
			Provider: client_mock.NewMockProvider(ctrl),
		})

		// Expect the callbacks in reverse-order of execution
		gomock.InOrder(
			plugin.EXPECT().PostRunHook(gomock.Any(), gomock.Any()),
			plugin.EXPECT().PostTestHook(gomock.Any(), gomock.Any()),
			plugin.EXPECT().PostBuildHook(gomock.Any(), gomock.Any()),
		)

		// Hook interceptors
		buildInterceptor := ps.BuildHooksInterceptor(streams)
		testInterceptor := ps.TestHooksInterceptor(streams)
		runInterceptor := ps.RunHooksInterceptor(streams)

		err := buildInterceptor(ctx, cmd, []string{}, func(ctx context.Context, cmd *cobra.Command, args []string) error {
			return testInterceptor(ctx, cmd, args, func(ctx context.Context, cmd *cobra.Command, args []string) error {
				return runInterceptor(ctx, cmd, args, func(ctx context.Context, cmd *cobra.Command, args []string) error {
					return nil
				})
			})
		})

		g.Expect(err).To(BeNil())
	})

	t.Run("executes plugin hooks in order plugins are added", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		// Setup
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout, Stderr: &stdout}
		ctx := context.Background()
		cmd := createInterceptorCommand()

		// Plugins to be invoked
		ps := NewPluginSystem().(*pluginSystem)
		plugin1 := plugin_mock.NewMockPlugin(ctrl)
		plugin2 := plugin_mock.NewMockPlugin(ctrl)
		ps.plugins.insert(&client.PluginInstance{
			Plugin:   plugin1,
			Provider: client_mock.NewMockProvider(ctrl),
		})
		ps.plugins.insert(&client.PluginInstance{
			Plugin:   plugin2,
			Provider: client_mock.NewMockProvider(ctrl),
		})

		// Expect the callbacks in reverse-order of execution, plugins in order added
		gomock.InOrder(
			plugin1.EXPECT().PostTestHook(gomock.Any(), gomock.Any()),
			plugin2.EXPECT().PostTestHook(gomock.Any(), gomock.Any()),
			plugin1.EXPECT().PostBuildHook(gomock.Any(), gomock.Any()),
			plugin2.EXPECT().PostBuildHook(gomock.Any(), gomock.Any()),
		)

		// Hook interceptors
		buildInterceptor := ps.BuildHooksInterceptor(streams)
		testInterceptor := ps.TestHooksInterceptor(streams)

		err := buildInterceptor(ctx, cmd, []string{}, func(ctx context.Context, cmd *cobra.Command, args []string) error {
			return testInterceptor(ctx, cmd, args, func(ctx context.Context, cmd *cobra.Command, args []string) error {
				return nil
			})
		})

		g.Expect(err).To(BeNil())
	})

	t.Run("returns pass nested interceptor errors to parent", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		// Setup
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout, Stderr: &stdout}
		ctx := context.Background()
		cmd := createInterceptorCommand()

		// Plugin to be invoked
		ps := NewPluginSystem().(*pluginSystem)
		plugin := plugin_mock.NewMockPlugin(ctrl)
		ps.plugins.insert(&client.PluginInstance{
			Plugin:   plugin,
			Provider: client_mock.NewMockProvider(ctrl),
		})

		// Expect the callbacks in reverse-order of execution
		gomock.InOrder(
			plugin.EXPECT().PostRunHook(gomock.Any(), gomock.Any()),
			plugin.EXPECT().PostTestHook(gomock.Any(), gomock.Any()),
			plugin.EXPECT().PostBuildHook(gomock.Any(), gomock.Any()),
		)

		// Hook interceptors
		buildInterceptor := ps.BuildHooksInterceptor(streams)
		testInterceptor := ps.TestHooksInterceptor(streams)
		runInterceptor := ps.RunHooksInterceptor(streams)

		// Return error in nested interceptor
		err := buildInterceptor(ctx, cmd, []string{}, func(ctx context.Context, cmd *cobra.Command, args []string) error {
			return testInterceptor(ctx, cmd, args, func(ctx context.Context, cmd *cobra.Command, args []string) error {
				return runInterceptor(ctx, cmd, args, func(ctx context.Context, cmd *cobra.Command, args []string) error {
					return fmt.Errorf("test error")
				})
			})
		})

		g.Expect(err).To(MatchError("test error"))
	})

	t.Run("parent interceptor errors override child errors", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		// Setup
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout, Stderr: &stdout}
		ctx := context.Background()
		cmd := createInterceptorCommand()

		// Plugin to be invoked
		ps := NewPluginSystem().(*pluginSystem)
		plugin := plugin_mock.NewMockPlugin(ctrl)
		ps.plugins.insert(&client.PluginInstance{
			Plugin:   plugin,
			Provider: client_mock.NewMockProvider(ctrl),
		})

		// Expect the callbacks in reverse-order of execution
		gomock.InOrder(
			plugin.EXPECT().PostRunHook(gomock.Any(), gomock.Any()),
			plugin.EXPECT().PostTestHook(gomock.Any(), gomock.Any()),
			plugin.EXPECT().PostBuildHook(gomock.Any(), gomock.Any()),
		)

		// Hook interceptors
		buildInterceptor := ps.BuildHooksInterceptor(streams)
		testInterceptor := ps.TestHooksInterceptor(streams)
		runInterceptor := ps.RunHooksInterceptor(streams)

		// Override error of nested interceptor
		err := buildInterceptor(ctx, cmd, []string{}, func(ctx context.Context, cmd *cobra.Command, args []string) error {
			return testInterceptor(ctx, cmd, args, func(ctx context.Context, cmd *cobra.Command, args []string) error {
				runInterceptor(ctx, cmd, args, func(ctx context.Context, cmd *cobra.Command, args []string) error {
					return fmt.Errorf("error 1")
				})
				return fmt.Errorf("error 2")
			})
		})

		g.Expect(err).To(MatchError("error 2"))
	})

	t.Run("ExitCode is not modified on error from interceptor", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		// Setup
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout, Stderr: &stdout}
		ctx := context.Background()
		cmd := createInterceptorCommand()

		ps := NewPluginSystem().(*pluginSystem)

		// Hook interceptor returning an error
		runInterceptor := ps.RunHooksInterceptor(streams)
		err := runInterceptor(ctx, cmd, []string{}, func(ctx context.Context, cmd *cobra.Command, args []string) error {
			return &aspecterrors.ExitError{
				Err:      fmt.Errorf("error 1"),
				ExitCode: 123,
			}
		})

		g.Expect(err).NotTo(BeNil())
		g.Expect(err.(*aspecterrors.ExitError).Err).To(MatchError("error 1"))
		g.Expect(err.(*aspecterrors.ExitError).ExitCode).To(Equal(123))
	})

	t.Run("ExitCode set to 1 on interceptor error of type aspecterrors.ExitError when a plugin returns an error", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		// Setup
		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout, Stderr: &stdout}
		ctx := context.Background()
		cmd := createInterceptorCommand()

		ps := NewPluginSystem().(*pluginSystem)

		// Plugin returning an error
		plugin := plugin_mock.NewMockPlugin(ctrl)
		plugin.EXPECT().
			PostRunHook(gomock.Any(), gomock.Any()).
			DoAndReturn(func(
				isInteractiveMode bool,
				promptRunner ioutils.PromptRunner,
			) error {
				return fmt.Errorf("plugin error")
			})
		ps.plugins.insert(&client.PluginInstance{
			Plugin:   plugin,
			Provider: client_mock.NewMockProvider(ctrl),
		})

		// Hook interceptors
		runInterceptor := ps.RunHooksInterceptor(streams)
		err := runInterceptor(ctx, cmd, []string{}, func(ctx context.Context, cmd *cobra.Command, args []string) error {
			return &aspecterrors.ExitError{
				Err:      fmt.Errorf("interceptor error"),
				ExitCode: 123,
			}
		})

		g.Expect(err).NotTo(BeNil())
		g.Expect(err.(*aspecterrors.ExitError).Err).To(MatchError("interceptor error"))
		g.Expect(err.(*aspecterrors.ExitError).ExitCode).To(Equal(1))
	})
}

func TestConfigure(t *testing.T) {
	t.Run("works when 0 plugins are found in config file", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout, Stderr: &stdout}

		ps := &pluginSystem{}

		err := ps.Configure(streams, nil)

		g.Expect(err).To(BeNil())
	})

	t.Run("creates and persists each plugin after invoking plugin.Setup()", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout, Stderr: &stdout}

		testPlugin := types.PluginConfig{
			Name:     "test plugin",
			From:     "...",
			Version:  "1.2.3",
			LogLevel: "debug",
		}
		testPlugin2 := types.PluginConfig{
			Name:     "test plugin2",
			From:     "...",
			Version:  "1.2.3",
			LogLevel: "debug",
		}

		p1 := plugin_mock.NewMockPlugin(ctrl)
		p1.EXPECT().Setup(gomock.Any())
		p2 := plugin_mock.NewMockPlugin(ctrl)
		p2.EXPECT().Setup(gomock.Any())

		factory := client_mock.NewMockFactory(ctrl)
		factory.EXPECT().New(testPlugin, streams).Return(
			&client.PluginInstance{
				Plugin:   p1,
				Provider: client_mock.NewMockProvider(ctrl),
			},
			nil,
		)
		factory.EXPECT().New(testPlugin2, streams).Return(
			&client.PluginInstance{
				Plugin:   p2,
				Provider: client_mock.NewMockProvider(ctrl),
			},
			nil,
		)

		ps := &pluginSystem{
			clientFactory: factory,
			plugins:       &PluginList{},
		}

		pluginConfig := []interface{}{
			map[string]interface{}{
				"name":      "test plugin",
				"from":      "...",
				"version":   "1.2.3",
				"log_level": "debug",
			},
			map[string]interface{}{
				"name":      "test plugin2",
				"from":      "...",
				"version":   "1.2.3",
				"log_level": "debug",
			},
		}

		err := ps.Configure(streams, pluginConfig)

		g.Expect(err).To(BeNil())
		g.Expect(ps.plugins.head.payload.Plugin).To(Equal(p1))
		g.Expect(ps.plugins.tail.payload.Plugin).To(Equal(p2))
	})

	t.Run("fails when a plugin initialization fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout, Stderr: &stdout}

		testPlugin := types.PluginConfig{
			Name:     "test plugin",
			From:     "...",
			Version:  "1.2.3",
			LogLevel: "debug",
		}
		testPlugin2 := types.PluginConfig{
			Name:     "test plugin2",
			From:     "...",
			Version:  "1.2.3",
			LogLevel: "debug",
		}

		p1 := plugin_mock.NewMockPlugin(ctrl)
		p1.EXPECT().Setup(gomock.Any())

		factory := client_mock.NewMockFactory(ctrl)
		factory.EXPECT().New(testPlugin, streams).Return(
			&client.PluginInstance{
				Plugin:   p1,
				Provider: client_mock.NewMockProvider(ctrl),
			},
			nil,
		)
		factory.EXPECT().New(testPlugin2, streams).Return(
			&client.PluginInstance{},
			errors.New("plugin New() error"),
		)

		ps := &pluginSystem{
			clientFactory: factory,
			plugins:       &PluginList{},
		}

		pluginConfig := []interface{}{
			map[string]interface{}{
				"name":      "test plugin",
				"from":      "...",
				"version":   "1.2.3",
				"log_level": "debug",
			},
			map[string]interface{}{
				"name":      "test plugin2",
				"from":      "...",
				"version":   "1.2.3",
				"log_level": "debug",
			},
		}

		err := ps.Configure(streams, pluginConfig)

		g.Expect(err).To(MatchError("failed to configure plugin system: plugin New() error"))
	})

	t.Run("fails when a plugin setup fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout, Stderr: &stdout}

		testPlugin := types.PluginConfig{
			Name:     "test plugin",
			From:     "...",
			Version:  "1.2.3",
			LogLevel: "debug",
		}
		testPlugin2 := types.PluginConfig{
			Name:     "test plugin2",
			From:     "...",
			Version:  "1.2.3",
			LogLevel: "debug",
		}

		p1 := plugin_mock.NewMockPlugin(ctrl)
		p1.EXPECT().Setup(gomock.Any())
		p2 := plugin_mock.NewMockPlugin(ctrl)
		p2.EXPECT().Setup(gomock.Any()).Return(errors.New("setup error"))

		factory := client_mock.NewMockFactory(ctrl)
		factory.EXPECT().New(testPlugin, streams).Return(
			&client.PluginInstance{
				Plugin:   p1,
				Provider: client_mock.NewMockProvider(ctrl),
			},
			nil,
		)
		factory.EXPECT().New(testPlugin2, streams).Return(
			&client.PluginInstance{
				Plugin:   p2,
				Provider: client_mock.NewMockProvider(ctrl),
			},
			nil,
		)

		ps := &pluginSystem{
			clientFactory: factory,
			plugins:       &PluginList{},
		}

		pluginConfig := []interface{}{
			map[string]interface{}{
				"name":      "test plugin",
				"from":      "...",
				"version":   "1.2.3",
				"log_level": "debug",
			},
			map[string]interface{}{
				"name":      "test plugin2",
				"from":      "...",
				"version":   "1.2.3",
				"log_level": "debug",
			},
		}

		err := ps.Configure(streams, pluginConfig)

		g.Expect(err).To(MatchError("failed to configure plugin system: setup error"))
	})

	t.Run("marshaled properties are passed to plugin.Setup", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout, Stderr: &stdout}

		propertiesMap := make(map[string]interface{})
		propertiesBytes, _ := yaml.Marshal(propertiesMap)
		setupConfig := plugin.NewSetupConfig(propertiesBytes)

		testPlugin := types.PluginConfig{
			Name:       "test plugin",
			From:       "...",
			Version:    "1.2.3",
			LogLevel:   "debug",
			Properties: propertiesMap,
		}

		p1 := plugin_mock.NewMockPlugin(ctrl)
		p1.EXPECT().Setup(setupConfig)

		factory := client_mock.NewMockFactory(ctrl)
		factory.EXPECT().New(testPlugin, streams).Return(
			&client.PluginInstance{
				Plugin:   p1,
				Provider: client_mock.NewMockProvider(ctrl),
			},
			nil,
		)

		ps := &pluginSystem{
			clientFactory: factory,
			plugins:       &PluginList{},
		}

		pluginConfig := []interface{}{
			map[string]interface{}{
				"name":       "test plugin",
				"from":       "...",
				"version":    "1.2.3",
				"log_level":  "debug",
				"properties": propertiesMap,
			},
		}

		err := ps.Configure(streams, pluginConfig)

		g.Expect(err).To(BeNil())
	})
}
