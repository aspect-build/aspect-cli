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

package configure

import (
	"context"
	"fmt"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func ProOnly(ctx context.Context, cmd *cobra.Command, args []string) error {
	return fmt.Errorf("The configure command is only available in Pro. Run 'aspect pro' to enable Pro features.")
}

func NewDefaultConfigureCmd() *cobra.Command {
	return NewConfigureCmd(ioutils.DefaultStreams, ProOnly)
}

func NewConfigureCmd(streams ioutils.Streams, cmdRunner interceptors.RunEContextFn) *cobra.Command {
	cmd := &cobra.Command{
		Use:     "configure",
		Short:   "Generate and update BUILD files",
		Long:    "Generates and updates BUILD files from sources for Typescript, Golang and Protobuf.",
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
