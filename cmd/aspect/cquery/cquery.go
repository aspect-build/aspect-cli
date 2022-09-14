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

package cquery

import (
	"context"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/cquery"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultCQueryCmd() *cobra.Command {
	return NewCQueryCommand(ioutils.DefaultStreams, bazel.New())
}

func NewCQueryCommand(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "cquery",
		Short: "Executes a cquery.",
		Long:  "Executes a query language expression over a specified subgraph of the build dependency graph using cquery.",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				q := cquery.New(streams, bzl, true)
				return q.Run(cmd, args)
			},
		),
	}

	return cmd
}
