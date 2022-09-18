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

package license

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/license"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultLicenseCmd() *cobra.Command {
	return NewLicenseCmd(ioutils.DefaultStreams)
}

func NewLicenseCmd(streams ioutils.Streams) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "license",
		Short: "Prints the license of this software.",
		Long:  "Prints the license of this software.",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			license.New(streams).Run,
		),
	}

	return cmd
}
