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

package init

import (
	"github.com/spf13/cobra"

	init_ "aspect.build/cli/pkg/aspect/init"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultCmd() *cobra.Command {
	return NewCmd(ioutils.DefaultStreams)
}

func NewCmd(streams ioutils.Streams) *cobra.Command {
	v := init_.New(streams)

	cmd := &cobra.Command{
		Use:   "init [folder]",
		Short: "Create a new Bazel workspace",
		Long: `Creates a Bazel workspace.

It stamps out commonly needed files to get started more quickly with a brand-new project.

Folder may be a new directory to create, or "." to use the current working directory.
If omitted, the user is prompted to supply a value.`,
		GroupID: "aspect",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			v.Run,
		),
	}
	return cmd
}
