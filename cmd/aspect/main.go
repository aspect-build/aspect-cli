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
	"fmt"
	"os"
	"time"

	"aspect.build/cli/cmd/aspect/root"
	"aspect.build/cli/pkg/aspect/root/config"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system"
	"github.com/spf13/viper"
)

func main() {
	start := time.Now()
	// Convenience for local development: under `bazel run <aspect binary target>` respect the
	// users working directory, don't run in the execroot
	if wd, exists := os.LookupEnv("BUILD_WORKING_DIRECTORY"); exists {
		_ = os.Chdir(wd)
	}

	bzl := bazel.WorkspaceFromWd

	if err := config.Load(viper.GetViper(), os.Args); err != nil {
		aspecterrors.HandleError(err)
	}

	streams := ioutils.DefaultStreams

	// Handle --version, -v and --bazel-version before re-entering and before initializing the
	// plugin system so these special "commands" are fast and don't require downloading a re-entrant
	// aspect or plugins before outputting their results.
	root.HandleVersionFlags(streams, os.Args[1:], bzl)

	// Re-enter another aspect if version running is not the configured version
	bazelVersion, reentered, err := bzl.HandleReenteringAspect(streams, os.Args[1:], root.CheckAspectLockVersionFlag(os.Args[1:]))
	if reentered {
		fmt.Println("REENTER")
		if err != nil {
			aspecterrors.HandleError(err)
		}
		os.Exit(0)
	}

	fmt.Println("0 ELAPSED", time.Since(start))

	err = bzl.InitializeBazelFlags(bazelVersion)
	if err != nil {
		aspecterrors.HandleError(err)
	}

	fmt.Println("A ELAPSED", time.Since(start))
	args, startupFlags, err := bazel.InitializeStartupFlags(os.Args[1:])

	if err != nil {
		aspecterrors.HandleError(err)
	}

	fmt.Println("AA ELAPSED", time.Since(start))
	if err = command(bzl, bazelVersion, streams, args, startupFlags); err != nil {
		aspecterrors.HandleError(err)
	}

	fmt.Println("BB ELAPSED", time.Since(start))
}

func command(bzl bazel.Bazel, version string, streams ioutils.Streams, args []string, startupFlags []string) error {

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
	if err := bzl.AddBazelFlags(cmd, version); err != nil {
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
