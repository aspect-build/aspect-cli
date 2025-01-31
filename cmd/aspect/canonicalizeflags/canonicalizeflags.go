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

package canonicalizeflags

import (
	"github.com/spf13/cobra"

	"github.com/aspect-build/aspect-cli/pkg/aspect/canonicalizeflags"
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
		Use:   "canonicalize-flags -- <bazel flags>",
		Short: "Present a list of bazel options in a canonical form",
		Long: `This command canonicalizes a list of bazel options.
		
This is useful when you need a unique key to group Bazel invocations by their flags.

Read [the Bazel canonicalize-flags documentation](https://bazel.build/docs/user-manual#canonicalize-flags)`,
		Example: `% aspect canonicalize-flags -- -k -c opt
--keep_going=1
--compilation_mode=opt`,
		DisableFlagsInUseLine: true,
		GroupID:               "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			canonicalizeflags.New(streams, bzl).Run,
		),
	}

	return cmd
}
