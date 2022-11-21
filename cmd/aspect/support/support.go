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

package support

import (
	"context"
	"fmt"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func ProOnly(ctx context.Context, cmd *cobra.Command, args []string) error {
	return fmt.Errorf("The support command is available in Aspect CLI Pro.. Run 'aspect pro' to enable Pro features.")
}

func NewDefaultCmd() *cobra.Command {
	return NewCmd(ioutils.DefaultStreams, ProOnly)
}

func NewCmd(streams ioutils.Streams, cmdRunner interceptors.RunEContextFn) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "support",
		Short: "Interactive, human-escalated support for Bazel problems",
		Long: `support collects recent Bazel invocations and collects relevant log files.

It then posts a message to a Slack channel on behalf of the user, posting the problem report in
a form that makes it easier for responders to understand the context and reproduce the problem.`,
		GroupID: "aspect",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			cmdRunner,
		),
	}
	return cmd
}
