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

package configure

import (
	"github.com/spf13/cobra"

	"github.com/aspect-build/aspect-cli/pkg/aspect/configure"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/interceptors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

func NewDefaultCmd() *cobra.Command {
	return NewCmd(ioutils.DefaultStreams)
}

func NewCmd(streams ioutils.Streams) *cobra.Command {
	return NewCmdWithConfigure(streams, configure.New(streams))
}
func NewCmdWithConfigure(streams ioutils.Streams, v *configure.Configure) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "configure",
		Short: "Auto-configure Bazel by updating BUILD files",
		Long: `configure generates and updates BUILD files from source code.

It is named after the "make configure" workflow which is typical in C++ projects, using
[autoconf](https://www.gnu.org/software/autoconf/).

configure is non-destructive: hand-edits to BUILD files are generally preserved.
You can use a ` + "`# keep`" + ` directive to force the tool to leave existing BUILD contents alone.
Run 'aspect help directives' for more documentation on directives.

So far these languages are supported:
- Go and Protocol Buffers, thanks to code from [gazelle]
- Python, thanks to code from [rules_python]
- JavaScript (including TypeScript)
- Kotlin (experimental, see https://github.com/aspect-build/aspect-cli/issues/474)
- Starlark, thanks to code from [bazel-skylib]

configure is based on [gazelle]. We are very grateful to the authors of that software.
The advantage of configure in Aspect CLI is that you don't need to compile the tooling before running it.

[gazelle]: https://github.com/bazelbuild/bazel-gazelle
[rules_python]: https://github.com/bazelbuild/rules_python/tree/main/gazelle
[bazel-skylib]: https://github.com/bazelbuild/bazel-skylib/tree/main/gazelle

To change the behavior of configure, you add "directives" to your BUILD files, which are comments
in a special syntax.
Run 'aspect help directives' or see https://docs.aspect.build/cli/help/directives for more info.
`,
		GroupID: "aspect",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			v.Run,
		),
	}

	// TODO: restrict to only valid values (see https://github.com/spf13/pflag/issues/236)
	cmd.Flags().String("mode", "fix", "Method for emitting merged BUILD files.\n\tfix: write generated and merged files to disk\n\tprint: print files to stdout\n\tdiff: print a unified diff")

	return cmd
}
