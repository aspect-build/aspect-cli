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

package lint

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/lint"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system"
)

func NewDefaultCmd(pluginSystem system.PluginSystem) *cobra.Command {
	return NewCmd(ioutils.DefaultStreams, pluginSystem, bazel.WorkspaceFromWd, []lint.LintResultsHandler{})
}

func NewCmd(streams ioutils.Streams, pluginSystem system.PluginSystem, bzl bazel.Bazel, lintHandlers []lint.LintResultsHandler) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "lint <target patterns>",
		Args:  cobra.MinimumNArgs(1),
		Short: "Run configured linters over the dependency graph.",
		Long: `Run linters and collect the reports they produce.

To setup linters, see the documentation on https://github.com/aspect-build/rules_lint

In addition to flags listed below, flags accepted by the 'bazel build' command are also accepted.
`,
		GroupID: "aspect",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
				pluginSystem.BESBackendSubscriberInterceptor(),
			},
			lint.New(streams, bzl, lintHandlers).Run,
		),
	}
	lint.AddFlags(cmd.Flags())

	for _, h := range lintHandlers {
		h.AddFlags(cmd.Flags())
	}

	return cmd
}
