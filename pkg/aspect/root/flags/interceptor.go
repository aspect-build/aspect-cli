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
	"strings"

	"github.com/spf13/cobra"
	"github.com/spf13/pflag"

	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func isFlagInFlagSet(flags *pflag.FlagSet, arg string) bool {
	var flag *pflag.Flag
	if arg[0] == '-' {
		if arg[1] == '-' {
			flag = flags.Lookup(strings.SplitN(arg[2:], "=", 2)[0])
		} else {
			for _, s := range arg[1:] {
				flag = flags.ShorthandLookup(string(s))
			}
		}
	}
	return flag != nil
}

func FlagsInterceptor(streams ioutils.Streams) interceptors.Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next interceptors.RunEContextFn) error {
		// DisableFlagParsing is true for commands that call out to Bazel. When DisableFlagParsing is true,
		// FlagsInterceptor will parse all flags but only remove aspect specific `--aspect:“ flags so that
		// all bazel flags can be forward to the bazel command.
		if cmd.DisableFlagParsing {
			parseArgs := make([]string, 0, len(args))
			forwardArgs := make([]string, 0, len(args))
			doubleDash := false
			for _, arg := range args {
				if arg == "--" {
					doubleDash = true
				}
				if !doubleDash {
					parseArgs = append(parseArgs, arg)
				}
				// Forward all flags to the bazel command except valid --aspect: flags that come before any
				// double dash (--). The valid --aspect: are all in rootCmd.PersistentFlags() so we can check
				// against that.
				if doubleDash || !isFlagInFlagSet(cmd.Root().PersistentFlags(), arg) {
					forwardArgs = append(forwardArgs, arg)
				}
			}
			cmd.DisableFlagParsing = false
			// Be tolerant of unknown flags since Bazel doesn't let us know what "alternate" flag names it
			// accepts and we don't want to error out if a user passes a valid "alternate" flag such as
			// --experimental_remote_grpc_log when the Bazel version being used reports only
			// --remote_grpc_log as a valid flag.
			cmd.FParseErrWhitelist = cobra.FParseErrWhitelist{UnknownFlags: true}
			if err := cmd.ParseFlags(parseArgs); err != nil {
				return err
			}
			cmd.DisableFlagParsing = true
			args = forwardArgs
		}
		return next(ctx, cmd, args)
	}
}
