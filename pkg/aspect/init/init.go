/*
 * Copyright 2023 Aspect Build Systems, Inc.
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

// init is a thin adapter for Aspect CLI to the Scaffold CLI: https://hay-kot.github.io/scaffold/
package init

import (
	"context"
	"fmt"
	"os"

	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/bazel/workspace"
	"aspect.build/cli/pkg/ioutils"

	"github.com/hay-kot/scaffold/app/commands"
	"github.com/hay-kot/scaffold/app/core/engine"
	"github.com/hay-kot/scaffold/app/scaffold/scaffoldrc"
	"github.com/rs/zerolog"
	"github.com/rs/zerolog/log"
	"github.com/spf13/cobra"
)

type Init struct {
	ioutils.Streams
}

func New(streams ioutils.Streams) *Init {
	return &Init{
		Streams: streams,
	}
}

func (runner *Init) Run(ctx context.Context, cmd *cobra.Command, args []string) error {
	wd, err := os.Getwd()
	if err != nil {
		return err
	}
	finder := workspace.DefaultFinder
	wr, err := finder.Find(wd)

	if err == nil {
		fmt.Printf("The current working directory is already inside a Bazel workspace rooted at %s.\n", wr)
		fmt.Println("Skipping new workspace creation...")
		// TODO: mention 'doctor' command when we have it
		// TODO: offer to add more stuff to the existing workspace, like language-specific support
		return nil
	}

	preset, _ := cmd.Flags().GetString("preset")
	isInteractiveMode, err := cmd.Root().PersistentFlags().GetBool(flags.AspectInteractiveFlagName)
	if err != nil {
		return err
	}

	ctrl := &commands.Controller{}
	// TODO: wire any relevant options from Aspect config.yaml
	rc := scaffoldrc.Default()
	ctrl.Prepare(engine.New(), rc)
	// TODO: set log level to match our context
	log.Logger = log.Level(zerolog.WarnLevel)
	// TODO: make the remote template configurable?
	args = append(args, "https://github.com/aspect-build/aspect-workflows-template.git")

	return ctrl.New(args, commands.FlagsNew{
		NoPrompt: !isInteractiveMode,
		Preset:   preset,
	})
}
