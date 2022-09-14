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

package version

import (
	"fmt"

	"aspect.build/cli/buildinfo"
	"github.com/spf13/cobra"
)

func NewVersionCmd() *cobra.Command {
	gnuFormat := false
	cmd := &cobra.Command{
		Use:   "version",
		Short: "Print the version of aspect CLI as well as tools it invokes.",
		Long:  `Prints version info on colon-separated lines, just like bazel does`,
		RunE: func(cmd *cobra.Command, args []string) error {
			bi := buildinfo.Current()
			format := buildinfo.ConventionalFormat
			if gnuFormat {
				format = buildinfo.GNUFormat
			}
			version := bi.CommandVersion("Aspect", format)
			if _, err := fmt.Println(version); err != nil {
				return err
			}
			return nil
		},
	}
	cmd.PersistentFlags().BoolVarP(
		&gnuFormat,
		"gnu_format",
		"",
		false,
		"format space-separated following GNU convention",
	)
	return cmd
}
