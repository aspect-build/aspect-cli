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

func NewDefaultFetchCmd() *cobra.Command {
	return NewFetchCmd(ioutils.DefaultStreams)
}

func NewFetchCmd(streams ioutils.Streams) *cobra.Command {
	cmd := &cobra.Command{
		Use:     "fetch",
		Short:   "Fetch external repositories that are prerequisites to the targets",
		Long:    "Fetches all external dependencies for the targets given.",
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
