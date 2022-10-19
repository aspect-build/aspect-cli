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

package shutdown

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspect/shutdown"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultShutdownCmd() *cobra.Command {
	return NewShutdownCmd(ioutils.DefaultStreams)
}

func NewShutdownCmd(streams ioutils.Streams) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "shutdown",
		Short: "Stop the bazel server",
		Long: `This command shuts down the memory resident bazel server process.

Documentation: <https://bazel.build/docs/user-manual#shutdown>`,
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			shutdown.New(streams).Run,
		),
	}

	return cmd
}
