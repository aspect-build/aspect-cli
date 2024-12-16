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

package clean

import (
	"github.com/spf13/cobra"

	"github.com/aspect-build/aspect-cli/pkg/aspect/clean"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/interceptors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

// NewDefaultCmd creates a new default clean cobra command.
func NewDefaultCmd() *cobra.Command {
	return NewCmd(ioutils.DefaultStreams, bazel.WorkspaceFromWd)
}

// NewCmd creates a new clean cobra command.
func NewCmd(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "clean [--expunge] [all]",
		Short: "Remove the output tree",
		Long: `Removes bazel-created output, including all object files, and bazel metadata.

Documentation: <https://bazel.build/docs/user-manual#clean>

clean deletes the output directories for all build configurations performed by
this Bazel instance, or the entire working tree created by this Bazel instance,
and resets internal caches.

If executed without any command-line options, then the output directory for all
configurations will be cleaned.

Recall that each Bazel instance is associated with a single workspace,
thus the clean command will delete all outputs from all builds you've
done with that Bazel instance in that workspace.

'clean all': Aspect CLI adds the ability to clean *all* Bazel workspaces on your machine,
by adding the argument "all".

NOTE: clean is primarily intended for reclaiming disk space for workspaces
that are no longer needed.
It causes all subsequent builds to be non-incremental.
If this is not your intent, consider these alternatives:

Do a one-off non-incremental build:
	bazel --output_base=$(mktemp -d) ...

Force repository rules to re-execute:
	bazel sync --configure

Workaround inconistent state:
	Bazel's incremental rebuilds are designed to be correct, so clean
	should never be required due to inconsistencies in the build.
	Such problems are fixable and these bugs are a high priority.
	If you ever find an incorrect incremental build, please file a bug report,
	and only use clean as a temporary workaround.`,
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			clean.NewDefault(streams, bzl).Run,
		),
	}

	return cmd
}
