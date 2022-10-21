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

package printaction

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/printaction"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultPrintActionCmd() *cobra.Command {
	return NewPrintActionCmd(ioutils.DefaultStreams)
}

func NewPrintActionCmd(streams ioutils.Streams) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "print_action <targets>",
		Short: "Print the command line args for compiling a file",
		Long: `Builds the specified targets and prints the extra actions for the given
targets. Right now, the targets have to be relative paths to source files,
and the --compile_one_dependency option has to be enabled.

This command accepts all valid options to 'build', and inherits defaults for
'build' from your .bazelrc.  If you don't use .bazelrc, don't forget to pass
all your 'build' options to 'print_action' too.

See 'bazel help target-syntax' for details and examples on how to
specify targets.`,
		Hidden:  true,
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			printaction.New(streams).Run,
		),
	}

	return cmd
}
