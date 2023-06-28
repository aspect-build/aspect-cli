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
	"github.com/spf13/pflag"

	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

// AddGlobalFlags will add Aspect specific flags to all cobra commands.
func AddGlobalFlags(cmd *cobra.Command, defaultInteractive bool) {
	cmd.PersistentFlags().Bool(AspectLockVersion, false, "Lock the version of the Aspect CLI. This prevents the Aspect CLI from downloading and running an different version of the Aspect CLI if one is specified in .bazeliskrc or the Aspect CLI config.")
	cmd.PersistentFlags().MarkHidden(AspectLockVersion)

	cmd.PersistentFlags().String(AspectConfigFlagName, "", fmt.Sprintf("User-specified Aspect CLI config file. /dev/null indicates that all further --%s flags will be ignored.", AspectConfigFlagName))

	RegisterNoableBool(cmd.PersistentFlags(), AspectSystemConfigFlagName, true, "Whether or not to look for the system config file at /etc/aspect/cli/config.yaml")
	cmd.PersistentFlags().MarkHidden(AspectSystemConfigFlagName)
	cmd.PersistentFlags().MarkHidden(NoFlagName(AspectSystemConfigFlagName))

	RegisterNoableBool(cmd.PersistentFlags(), AspectWorkspaceConfigFlagName, true, "Whether or not to look for the workspace config file at $workspace/.aspect/cli/config.yaml")
	cmd.PersistentFlags().MarkHidden(AspectWorkspaceConfigFlagName)
	cmd.PersistentFlags().MarkHidden(NoFlagName(AspectWorkspaceConfigFlagName))

	RegisterNoableBool(cmd.PersistentFlags(), AspectHomeConfigFlagName, true, "Whether or not to look for the home config file at $HOME/.aspect/cli/config.yaml")
	cmd.PersistentFlags().MarkHidden(AspectHomeConfigFlagName)
	cmd.PersistentFlags().MarkHidden(NoFlagName(AspectHomeConfigFlagName))

	cmd.PersistentFlags().Bool(AspectInteractiveFlagName, defaultInteractive, "Interactive mode (e.g. prompts for user input)")
}

func extractUnknownArgs(flags *pflag.FlagSet, args []string) []string {
	unknownArgs := []string{}

	for i := 0; i < len(args); i++ {
		arg := args[i]
		var flag *pflag.Flag
		if arg[0] == '-' {
			if arg[1] == '-' {
				// --; bail
				if len(arg) == 2 {
					unknownArgs = append(unknownArgs, args[i:]...)
					break
				}
				flag = flags.Lookup(strings.SplitN(arg[2:], "=", 2)[0])
			} else {
				for _, s := range arg[1:] {
					flag = flags.ShorthandLookup(string(s))
					if flag == nil {
						break
					}
				}
			}
		}
		if flag != nil {
			if flag.NoOptDefVal == "" && i+1 < len(args) && flag.Value.String() == args[i+1] {
				i++
			}
			continue
		}
		unknownArgs = append(unknownArgs, arg)
	}
	return unknownArgs
}

// FlagsIntercepor will parse the incoming flags and remove any aspect specific flags or bazel
// startup flags from the list of args.
func FlagsInterceptor(streams ioutils.Streams) interceptors.Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next interceptors.RunEContextFn) error {

		if cmd.DisableFlagParsing {
			cmd.DisableFlagParsing = false
			args = extractUnknownArgs(cmd.InheritedFlags(), args)
			if err := cmd.ParseFlags(args); err != nil {
				return err
			}
		}

		for _, arg := range args {
			if arg == "--" {
				break
			}
			if strings.HasPrefix(arg, "--aspect:") {
				return fmt.Errorf("unknown flag: %s", arg)
			}
		}

		return next(ctx, cmd, args)
	}
}
