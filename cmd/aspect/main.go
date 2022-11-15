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

	"aspect.build/cli/cmd/aspect/root"
	"aspect.build/cli/pkg/aspect/root/config"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system"
)

func main() {
	// Convenience for local development: under `bazel run <aspect binary target>` respect the
	// users working directory, don't run in the execroot
	if wd, exists := os.LookupEnv("BUILD_WORKING_DIRECTORY"); exists {
		_ = os.Chdir(wd)
	}

	bzl, err := bazel.FindFromWd()
	if err != nil {
		aspecterrors.HandleError(err)
	}

	if err := config.Load(os.Args); err != nil {
		aspecterrors.HandleError(err)
	}

	streams := ioutils.DefaultStreams

	// Re-enter another aspect if version running is not the configured version
	reentered, exitCode, err := bzl.MaybeReenterAspect(streams, os.Args[1:])
	if reentered {
		if err != nil {
			aspecterrors.HandleError(err)
		}
		os.Exit(exitCode)
	}

	// Handle --version and -v before initializing the plugin system so these special
	// "commands" are fast and don't require download plugins before output the version.
	root.MaybeAspectVersionFlag(streams, os.Args[1:])

	argsWithoutStartupFlags, err := bzl.InitializeStartupFlags(os.Args)
	if err != nil {
		aspecterrors.HandleError(err)
	}
	os.Args = argsWithoutStartupFlags

	pluginSystem := system.NewPluginSystem()
	if err := pluginSystem.Configure(streams); err != nil {
		aspecterrors.HandleError(err)
	}

	defer pluginSystem.TearDown()

	cmd := root.NewDefaultRootCmd(pluginSystem)

	// Run this command after all bazel verbs have been added to "cmd".
	if err := bazel.AddBazelFlags(cmd); err != nil {
		aspecterrors.HandleError(err)
	}

	if err := pluginSystem.RegisterCustomCommands(cmd); err != nil {
		aspecterrors.HandleError(err)
	}

	if err := cmd.ExecuteContext(context.Background()); err != nil {
		aspecterrors.HandleError(err)
	}
}
