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

package analyzeprofile

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/analyzeprofile"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultAnalyzeProfileCmd() *cobra.Command {
	return NewAnalyzeProfileCmd(ioutils.DefaultStreams)
}

func NewAnalyzeProfileCmd(streams ioutils.Streams) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "analyze-profile",
		Short: "Analyzes build profile data.",
		Long: `Analyzes build profile data for the given profile data files.

Analyzes each specified profile data file and prints the results.  The
input files must have been produced by the 'bazel build
--profile=file' command.

By default, a summary of the analysis is printed.  For post-processing
with scripts, the --dump=raw option is recommended, causing this
command to dump profile data in easily-parsed format.`,
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			analyzeprofile.New(streams).Run,
		),
	}

	return cmd
}
