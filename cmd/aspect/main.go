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

package main

import (
	"context"
	"os"

	"github.com/aspect-build/aspect-cli/cmd/aspect/root"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/config"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/hints"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/aspect-build/aspect-cli/pkg/plugin/system"
	"github.com/spf13/viper"
)

func main() {
	// Convenience for local development: under `bazel run <aspect binary target>` respect the
	// users working directory, don't run in the execroot
	if wd, exists := os.LookupEnv("BUILD_WORKING_DIRECTORY"); exists {
		_ = os.Chdir(wd)
	}

	bzl := bazel.WorkspaceFromWd

	// Load Aspect CLI config.yaml
	if err := config.Load(viper.GetViper(), os.Args); err != nil {
		aspecterrors.HandleError(err)
	}

	streams := ioutils.DefaultStreams

	// Handle --version, -v and --bazel-version before re-entering and before initializing the
	// plugin system so these special "commands" are fast and don't require downloading a re-entrant
	// aspect or plugins before outputting their results.
	root.HandleVersionFlags(streams, os.Args[1:], bzl)

	// Re-enter another aspect if version running is not the configured version
	reentered, err := bzl.HandleReenteringAspect(streams, os.Args[1:], root.CheckAspectLockVersionFlag(os.Args[1:]))
	if reentered {
		if err != nil {
			aspecterrors.HandleError(err)
		}
		os.Exit(0)
	}

	err = bzl.InitializeBazelFlags()
	if err != nil {
		aspecterrors.HandleError(err)
	}

	args, startupFlags, err := bazel.InitializeStartupFlags(os.Args[1:])
	if err != nil {
		aspecterrors.HandleError(err)
	}

	h := hints.New()

	// Configure hints from Aspect CLI config.yaml 'hints' attribute
	if err := h.Configure(viper.Get("hints")); err != nil {
		aspecterrors.HandleError(err)
	}

	// Attach hints from Stdout and Stderr streams
	if err := h.Attach(); err != nil {
		aspecterrors.HandleError(err)
	}

	err = command(bzl, streams, args, startupFlags)

	// Detach hints from Stdout and Stderr streams
	h.Detach()

	// Print hints
	h.PrintHints(os.Stderr)

	// Handle command errors
	if err != nil {
		aspecterrors.HandleError(err)
	}
}

func command(bzl bazel.Bazel, streams ioutils.Streams, args []string, startupFlags []string) error {

	pluginsConfig := viper.Get("plugins")
	pluginSystem := system.NewPluginSystem()

	if !root.CheckAspectDisablePluginsFlag(args) {
		if err := pluginSystem.Configure(streams, pluginsConfig); err != nil {
			return err
		}
	}

	defer pluginSystem.TearDown()

	cmd := root.NewDefaultCmd(pluginSystem)

	// Run this command after all bazel verbs have been added to "cmd".
	if err := bzl.AddBazelFlags(cmd); err != nil {
		return err
	}

	if err := pluginSystem.RegisterCustomCommands(cmd, startupFlags); err != nil {
		return err
	}

	os.Args = append(os.Args[0:1], args...)

	if err := cmd.ExecuteContext(context.Background()); err != nil {
		return err
	}

	return nil
}
