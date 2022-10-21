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

	"aspect.build/cli/pkg/aspect/canonicalizeflags"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultCanonicalizeFlagsCmd() *cobra.Command {
	return NewCanonicalizeFlagsCmd(ioutils.DefaultStreams)
}

func NewCanonicalizeFlagsCmd(streams ioutils.Streams) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "canonicalize-flags -- <bazel flags>",
		Short: "Present a list of bazel options in a canonical form",
		Long: `This command canonicalizes a list of bazel options.
		
This is useful when you need a unique key to group Bazel invocations by their flags.

Documentation: <https://bazel.build/docs/user-manual#canonicalize-flags>`,
		Example: `% aspect canonicalize-flags -- -k -c opt
--keep_going=1
--compilation_mode=opt`,
		DisableFlagsInUseLine: true,
		GroupID:               "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			canonicalizeflags.New(streams).Run,
		),
	}

	return cmd
}
