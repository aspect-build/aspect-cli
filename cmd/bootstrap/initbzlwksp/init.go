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

package initbzlwksp

import (
	"aspect.build/cli/pkg/bazel"
	"github.com/spf13/cobra"
)

func NewInitCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "init",
		Short: "Initialize a Bazel workspace to use the aspect CLI.",
		Long:  `Initializes a Bazel workspace to use the aspect CLI.`,
		Args:  cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			var path string
			if len(args) > 0 {
				path = args[0]
			} else {
				b := bazel.New()
				wr, err := b.WorkspaceRoot()
				if err != nil {
					return err
				}
				path = bazel.VersionPath(wr)
			}

			version, err := bazel.SafeVersionFromFile(path)
			if err != nil {
				return err
			}
			version.InitAspect()
			return version.WriteToFile(path)
		},
	}
}
