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

func NewDefaultCmd() *cobra.Command {
	return NewCmd(ioutils.DefaultStreams)
}

func NewCmd(streams ioutils.Streams) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "analyze-profile <command.profile.gz>",
		Args:  cobra.MinimumNArgs(1),
		Short: "Analyze build profile data",
		Long: `Analyzes build profile data for the given profile data file(s).

Analyzes each specified profile data file and prints the results.
The profile is commonly written to ` + "`$(bazel info output_base)/command.profile.gz`" + `
after a build command completes.
You can use the ` + "`--profile=<file>`" + ` flag to supply an alternative path where the profile is written.

This command just dumps profile data to stdout. To inspect a profile you may want to use a GUI
instead, such as the ` + "`chrome//:tracing`" + ` interface built into Chromium / Google Chrome, or
<https://ui.perfetto.dev/>.

By default, a summary of the analysis is printed.  For post-processing
with scripts, the ` + "`--dump=raw`" + ` option is recommended, causing this
command to dump profile data in easily-parsed format.`,
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			analyzeprofile.New(streams).Run,
		),
	}

	return cmd
}
