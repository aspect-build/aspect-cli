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

package fetch

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/fetch"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultCmd() *cobra.Command {
	return NewCmd(ioutils.DefaultStreams)
}

func NewCmd(streams ioutils.Streams) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "fetch <target patterns>",
		Args:  cobra.MinimumNArgs(1),
		Short: "Fetch external repositories that are prerequisites to the targets",
		Long: `Fetches all external dependencies for the targets given.

Note that Bazel uses the term "fetch" to mean both downloading remote files, and also running local
installation commands declared by rules for these external files.

Documentation: <https://bazel.build/run/build#fetching-external-dependencies>

If you observe fetching that should not be needed to build the
requested targets, this may indicate an "eager fetch" bug in some ruleset you rely on.
Read more: <https://blog.aspect.dev/avoid-eager-fetches>`,
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			fetch.New(streams).Run,
		),
	}

	return cmd
}
