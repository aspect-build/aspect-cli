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

	"github.com/aspect-build/aspect-cli/pkg/aspect/info"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/hints"
	"github.com/aspect-build/aspect-cli/pkg/interceptors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

func NewDefaultCmd() *cobra.Command {
	return NewCmd(
		ioutils.DefaultStreams,
		hints.DefaultStreams,
		bazel.WorkspaceFromWd,
	)
}

func NewCmd(streams ioutils.Streams, hstreams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "info [keys]",
		Short: "Display runtime info about the bazel server",
		Long: `Displays information about the state of the bazel process in the
form of several "key: value" pairs.  This includes the locations of
several output directories.  Because some of the
values are affected by the options passed to 'bazel build', the
info command accepts the same set of options.

Read [the Bazel info documentation](https://bazel.build/docs/user-manual#info)

If arguments are specified, each should be one of the keys (e.g. "bazel-bin").
In this case only the value(s) for those keys will be printed.

If --show_make_env is specified, the output includes the set of key/value
pairs in the "Make" environment, accessible within BUILD files.

One or more of the following keys can be supplied as arguments, such as 'info bazel-bin'.
When no arguments are given, most key/values are printed.

| Key                     | Description                                                               |
| ----------------------- | ------------------------------------------------------------------------- |
| bazel-bin               | Configuration dependent directory for binaries.                           |
| bazel-genfiles          | Configuration dependent directory for generated files.                    |
| bazel-testlogs          | Configuration dependent directory for logs from a test run.               |
| build-language          | Print a binary-encoded protocol buffer with the build language structure. |
| character-encoding      | Information about the character encoding used by the running JVM.         |
| client-env              | The specifications to freeze the current client environment. [^1]         |
| command_log             | Location of the log containing the output from the build commands.        |
| committed-heap-size     | Amount of memory in bytes that is committed for the JVM to use.           |
| default-package-path    | The default package path.                                                 |
| execution_root          | A directory that makes all input and output files visible to the build.   |
| gc-count                | Number of garbage collection runs.                                        |
| gc-time                 | The approximate accumulated time spend on garbage collection.             |
| install_base            | The installation base directory.                                          |
| java-home               | Location of the current Java runtime.                                     |
| java-runtime            | Name and version of the current Java runtime environment.                 |
| java-vm                 | Name and version of the current Java virtual machine.                     |
| max-heap-size           | Maximum amount of memory in bytes that can be used for memory management. |
| output_base             | A directory for shared bazel state. [^2]                                  |
| output_path             | The output directory.                                                     |
| package_path            | The search path for resolving package labels.                             |
| peak-heap-size          | The peak amount of used memory in bytes after any call to System.gc().    |
| release                 | bazel release identifier.                                                 |
| repository_cache        | The location of the repository download cache used.                       |
| server_log              | The bazel server log path.                                                |
| server_pid              | The bazel process id.                                                     |
| starlark-semantics      | The effective set of Starlark semantics option values.                    |
| used-heap-size          | The amount of used memory in bytes. [^3]                                  |
| used-heap-size-after-gc | The amount of used memory in bytes after a call to System.gc().           |
| workspace               | The working directory of the server.                                      |

[^1]:
    The output can be added to the project-specific rc file. See
    https://bazel.build/designs/2016/06/21/environment.html

[^2]: As well as tool and strategy specific subdirectories.
[^3]:
    Note that this is not a good indicator of the actual memory use, as it
    includes any remaining inaccessible memory.
`,
		GroupID: "built-in",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			info.New(streams, hstreams, bzl).Run,
		),
	}
	return cmd
}
