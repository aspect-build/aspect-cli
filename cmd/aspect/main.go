/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package main

import (
	"context"
	"errors"
	"fmt"
	"os"

	"aspect.build/cli/cmd/aspect/root"
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

	pluginSystem := system.NewPluginSystem()
	if err := pluginSystem.Configure(ioutils.DefaultStreams); err != nil {
		fmt.Fprintln(os.Stderr, "Error:", err)
		os.Exit(1)
	}

	defer pluginSystem.TearDown()

	cmd := root.NewDefaultRootCmd(pluginSystem)
	cmd = pluginSystem.AddCustomCommands(cmd)
	if err := cmd.ExecuteContext(context.Background()); err != nil {
		var exitErr *aspecterrors.ExitError
		if errors.As(err, &exitErr) {
			if exitErr.Err != nil {
				fmt.Fprintln(os.Stderr, "Error:", err)
			}
			os.Exit(exitErr.ExitCode)
		}

		fmt.Fprintln(os.Stderr, "Error:", err)
		os.Exit(1)
	}
}
