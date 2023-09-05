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

	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

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
				if doubleDash || !strings.HasPrefix(arg, "--aspect:") {
					forwardArgs = append(forwardArgs, arg)
				}
			}
			cmd.DisableFlagParsing = false
			if err := cmd.ParseFlags(parseArgs); err != nil {
				return err
			}
			cmd.DisableFlagParsing = true
			args = forwardArgs
		}
		return next(ctx, cmd, args)
	}
}
