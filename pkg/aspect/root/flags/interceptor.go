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

package flags

import (
	"context"
	"fmt"
	"strings"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

// AddGlobalFlags will add aspect specfic flags to all cobra commands.
func AddGlobalFlags(cmd *cobra.Command, defaultInteractive bool) {
	cmd.PersistentFlags().String(AspectConfigFlagName, "", fmt.Sprintf("User-specified Aspect CLI config file. /dev/null indicates that all further --%s flags will be ignored.", AspectConfigFlagName))
	RegisterNoableBool(cmd.PersistentFlags(), AspectWorkspaceConfigFlagName, true, "Whether or not to look for the workspace config file at $workspace/.aspect/cli/config.yaml")
	cmd.PersistentFlags().MarkHidden(AspectWorkspaceConfigFlagName)
	cmd.PersistentFlags().MarkHidden(NoNameAspect(AspectWorkspaceConfigFlagName))
	RegisterNoableBool(cmd.PersistentFlags(), AspectHomeConfigFlagName, true, "Whether or not to look for the home config file at $HOME/.aspect/cli/config.yaml")
	cmd.PersistentFlags().MarkHidden(AspectHomeConfigFlagName)
	cmd.PersistentFlags().MarkHidden(NoNameAspect(AspectHomeConfigFlagName))
	cmd.PersistentFlags().Bool(AspectInteractiveFlagName, defaultInteractive, "Interactive mode (e.g. prompts for user input)")
}

// FlagsIntercepor will parse the incoming flags and remove any aspect specific flags or bazel
// startup flags from the list of args.
func FlagsInterceptor(streams ioutils.Streams) interceptors.Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next interceptors.RunEContextFn) error {

		if cmd.DisableFlagParsing {
			cmd.DisableFlagParsing = false

			if err := cmd.ParseFlags(args); err != nil {
				return err
			}
		}

		// Remove "--aspect:*" flags from the list of args. These should be accessed via cmd.Flags()
		updatedArgs := make([]string, 0)
		for i := 0; i < len(args); i++ {
			if strings.HasPrefix(args[i], "--aspect:") {
				continue
			}

			updatedArgs = append(updatedArgs, args[i])
		}

		return next(ctx, cmd, updatedArgs)
	}
}
