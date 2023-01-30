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

package sync

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspect/sync"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultCmd() *cobra.Command {
	return NewCmd(ioutils.DefaultStreams, bazel.WorkspaceFromWd)
}

func NewCmd(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "sync",
		Short: "Sync all repositories specified in the WORKSPACE file",
		Long: `Ensures that all Starlark repository rules of the top-level WORKSPACE
file are called.

NOTE: This command is still very experimental and the precise semantics
will change in the near future.`,
		Hidden:  true, // This command is documented as "very experimental"
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			sync.New(streams, bzl).Run,
		),
	}

	return cmd
}
