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

package dump

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/dump"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultDumpCmd() *cobra.Command {
	return NewDumpCmd(ioutils.DefaultStreams)
}

func NewDumpCmd(streams ioutils.Streams) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "dump",
		Short: "Dump the internal state of the bazel server process",
		Long: `Dumps the internal state of the bazel server process.

This command is provided as an aid to debugging, not as a stable interface, so
users should not try to parse the output; instead, use 'query' or 'info' for
this purpose.`,
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			dump.New(streams).Run,
		),
	}

	return cmd
}
