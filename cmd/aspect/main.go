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
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system"
)

func main() {
	// Detect whether we are being run as a tools/bazel wrapper (look for BAZEL_REAL in the environment)
	// If so,
	//     Is this a bazel-native command? just call through to bazel without touching the arguments for now
	//     Is this an aspect-custom command? (like `outputs`) then write an implementation
	// otherwise,
	//     we are installing ourselves. Check with the user they intended to do that.
	//     then create
	//         - a WORKSPACE file, ask the user for the repository name if interactive
	//     ask the user if they want to install for all users of the workspace, if so
	//         - tools/bazel file and put our bootstrap code in there
	//

	// Convenience for local development: under `bazel run //:aspect` respect the
	// users working directory, don't run in the execroot
	if wd, exists := os.LookupEnv("BUILD_WORKING_DIRECTORY"); exists {
		_ = os.Chdir(wd)
	}

	pluginSystem := system.NewDefaultPluginSystem()
	if err := pluginSystem.Configure(ioutils.DefaultStreams); err != nil {
		aspecterrors.HandleError(err)
	}

	defer pluginSystem.TearDown()

	cmd := root.NewDefaultRootCmd(pluginSystem)

	// Run this command after all bazel verbs have been added to "cmd".
	if err := flags.AddBazelFlags(cmd); err != nil {
		aspecterrors.HandleError(err)
	}

	if err := pluginSystem.RegisterCustomCommands(cmd); err != nil {
		aspecterrors.HandleError(err)
	}

	if err := cmd.ExecuteContext(context.Background()); err != nil {
		aspecterrors.HandleError(err)
	}
}
