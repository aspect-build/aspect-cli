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

package outputs

import (
	"github.com/spf13/cobra"

	"github.com/aspect-build/aspect-cli/pkg/aspect/outputs"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/interceptors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

func NewDefaultCmd() *cobra.Command {
	return NewCmd(ioutils.DefaultStreams, bazel.WorkspaceFromWd)
}

func NewCmd(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "outputs <expression> [mnemonic]",
		Short: "Print paths to declared output files",
		Long: `Queries for the outputs declared by actions generated by the given target or the target(s) in the given query expression.
 
Prints each output file on a line, with the mnemonic of the action that produces it,
followed by a path to the file, relative to the workspace root.

You can optionally provide an extra argument, which is a filter on the mnemonic.

'ExecutableHash' is a special value for the mnemonic. This combines the ExecutableSymlink and
SourceSymlinkManifest mnemonics, then hashes the outputs of these two. This provides a good hash
for an executable target to determine if it has changed.`,
		Example: `# Show all outputs of the //cli/pro target, which is a go_binary:

% aspect outputs //cli/pro

GoCompilePkg bazel-out/k8-fastbuild/bin/cli/pro/pro.a
GoCompilePkg bazel-out/k8-fastbuild/bin/cli/pro/pro.x
GoLink bazel-out/k8-fastbuild/bin/cli/pro/pro_/pro
SourceSymlinkManifest bazel-out/k8-fastbuild/bin/cli/pro/pro_/pro.runfiles_manifest
SymlinkTree bazel-out/k8-fastbuild/bin/cli/pro/pro_/pro.runfiles/MANIFEST
Middleman bazel-out/k8-fastbuild/internal/_middlemen/cli_Spro_Spro_U_Spro-runfiles

# Show just the output of the GoLink action, which is the executable produced by a go_binary:

% aspect outputs //cli/pro GoLink
bazel-out/k8-fastbuild/bin/cli/pro/pro_/pro

# Show the outputs of all targets that contain the 'deliverable' tag

% aspect outputs 'attr("tags", "\bdeliverable\b", //...)'

ExecutableSymlink bazel-out/darwin-fastbuild/bin/cli/release
SourceSymlinkManifest bazel-out/darwin-fastbuild/bin/cli/release.runfiles_manifest
SymlinkTree bazel-out/darwin-fastbuild/bin/cli/release.runfiles/MANIFEST
Middleman bazel-out/darwin-fastbuild/internal/_middlemen/cli_Srelease-runfiles`,
		GroupID: "aspect",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			outputs.New(streams, bzl).Run,
		),
	}

	outputs.AddFlags(cmd.Flags())

	return cmd
}
