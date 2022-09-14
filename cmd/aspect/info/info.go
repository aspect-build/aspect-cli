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

package info

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/info"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultInfoCmd() *cobra.Command {
	return NewInfoCmd(ioutils.DefaultStreams)
}

func NewInfoCmd(streams ioutils.Streams) *cobra.Command {
	v := info.New(streams)

	cmd := &cobra.Command{
		Use:   "info",
		Short: "Displays runtime info about the bazel server.",
		Long: `Displays information about the state of the bazel process in the
form of several "key: value" pairs.  This includes the locations of
several output directories.  Because some of the
values are affected by the options passed to 'bazel build', the
info command accepts the same set of options.

A single non-option argument may be specified (e.g. "bazel-bin"), in
which case only the value for that key will be printed.

If --show_make_env is specified, the output includes the set of key/value
pairs in the "Make" environment, accessible within BUILD files.

The full list of keys and the meaning of their values is documented in
the bazel User Manual, and can be programmatically obtained with
'bazel help info-keys'.

See also 'bazel version' for more detailed bazel version
information.`,
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			v.Run,
		),
	}

	cmd.PersistentFlags().BoolVarP(&v.ShowMakeEnv, "show_make_env", "", false, `include the set of key/value pairs in the "Make" environment,
accessible within BUILD files`)
	return cmd
}
