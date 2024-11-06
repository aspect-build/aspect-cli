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

package root

import (
	"fmt"
	"os"

	"github.com/fatih/color"
	"github.com/mattn/go-isatty"
	"github.com/spf13/cobra"

	"aspect.build/cli/buildinfo"
	"aspect.build/cli/cmd/aspect/analyzeprofile"
	"aspect.build/cli/cmd/aspect/aquery"
	"aspect.build/cli/cmd/aspect/build"
	"aspect.build/cli/cmd/aspect/canonicalizeflags"
	"aspect.build/cli/cmd/aspect/clean"
	"aspect.build/cli/cmd/aspect/config"
	"aspect.build/cli/cmd/aspect/configure"
	"aspect.build/cli/cmd/aspect/coverage"
	"aspect.build/cli/cmd/aspect/cquery"
	"aspect.build/cli/cmd/aspect/docs"
	"aspect.build/cli/cmd/aspect/dump"
	"aspect.build/cli/cmd/aspect/fetch"
	"aspect.build/cli/cmd/aspect/help"
	"aspect.build/cli/cmd/aspect/info"
	init_ "aspect.build/cli/cmd/aspect/init"
	"aspect.build/cli/cmd/aspect/license"
	"aspect.build/cli/cmd/aspect/lint"
	"aspect.build/cli/cmd/aspect/mobileinstall"
	"aspect.build/cli/cmd/aspect/mod"
	"aspect.build/cli/cmd/aspect/outputs"
	"aspect.build/cli/cmd/aspect/print"
	"aspect.build/cli/cmd/aspect/printaction"
	"aspect.build/cli/cmd/aspect/query"
	"aspect.build/cli/cmd/aspect/run"
	"aspect.build/cli/cmd/aspect/shutdown"
	"aspect.build/cli/cmd/aspect/sync"
	"aspect.build/cli/cmd/aspect/test"
	"aspect.build/cli/cmd/aspect/version"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system"
)

var (
	boldCyan = color.New(color.FgCyan, color.Bold)
	faint    = color.New(color.Faint)
)

func NewDefaultCmd(pluginSystem system.PluginSystem) *cobra.Command {
	defaultInteractive := isatty.IsTerminal(os.Stdin.Fd()) || isatty.IsCygwinTerminal(os.Stdin.Fd())
	return NewCmd(ioutils.DefaultStreams, pluginSystem, defaultInteractive)
}

func CheckAspectLockVersionFlag(args []string) bool {
	for _, arg := range args {
		if arg == "--"+flags.AspectLockVersion+"=false" {
			return false
		}
		if arg == "--"+flags.AspectLockVersion+"=true" || arg == "--"+flags.AspectLockVersion {
			return true
		}
	}
	return flags.AspectLockVersionDefault()
}

func CheckAspectDisablePluginsFlag(args []string) bool {
	for _, arg := range args {
		if arg == "--"+flags.AspectDisablePluginsFlagName {
			return true
		}
	}
	return false
}

func HandleVersionFlags(streams ioutils.Streams, args []string, bzl bazel.Bazel) {
	if len(args) == 1 && (args[0] == "--version" || args[0] == "-v") {
		fmt.Fprintf(streams.Stdout, "%s %s\n", buildinfo.Current().GnuName(), buildinfo.Current().Version())
		os.Exit(0)
	}
	if len(args) == 1 && args[0] == "--bazel-version" {
		version, err := bzl.BazelDashDashVersion()
		if err != nil {
			aspecterrors.HandleError(err)
		}
		fmt.Fprint(streams.Stdout, version)
		os.Exit(0)
	}
}

func NewCmd(
	streams ioutils.Streams,
	pluginSystem system.PluginSystem,
	defaultInteractive bool,
) *cobra.Command {
	cmd := &cobra.Command{
		Use:           "aspect",
		Short:         "Aspect CLI",
		SilenceUsage:  true,
		SilenceErrors: true,
		Long:          boldCyan.Sprintf("Aspect CLI is a better frontend for running bazel"),
		// Suppress timestamps in generated Markdown, for determinism
		DisableAutoGenTag: true,
		Version:           buildinfo.Current().Version(),
	}

	// Fallback version template incase it is not handled by HandleVersionFlags
	cmd.SetVersionTemplate(fmt.Sprintf("%s %s\n", buildinfo.Current().GnuName(), buildinfo.Current().Version()))

	flags.AddGlobalFlags(cmd, defaultInteractive)
	cmd.AddGroup(&cobra.Group{ID: "common", Title: "Common Bazel Commands:"})
	cmd.AddGroup(&cobra.Group{ID: "aspect", Title: "Commands only in Aspect CLI:"})
	cmd.AddGroup(&cobra.Group{ID: "plugin", Title: "Custom Commands from Plugins:"})
	cmd.AddGroup(&cobra.Group{ID: "built-in", Title: "Other Bazel Built-in Commands:"})

	// ### Child commands
	// IMPORTANT: when adding a new command, also update the COMMAND_LIST list in /docs/command_list.bzl
	cmd.AddCommand(analyzeprofile.NewDefaultCmd())
	cmd.AddCommand(aquery.NewDefaultCmd())
	cmd.AddCommand(build.NewDefaultCmd(pluginSystem))
	cmd.AddCommand(canonicalizeflags.NewDefaultCmd())
	cmd.AddCommand(clean.NewDefaultCmd())
	cmd.AddCommand(config.NewDefaultCmd())
	cmd.AddCommand(coverage.NewDefaultCmd(pluginSystem))
	cmd.AddCommand(cquery.NewDefaultCmd())
	cmd.AddCommand(dump.NewDefaultCmd())
	cmd.AddCommand(fetch.NewDefaultCmd())
	cmd.AddCommand(docs.NewDefaultCmd())
	cmd.AddCommand(info.NewDefaultCmd())
	cmd.AddCommand(init_.NewDefaultCmd())
	cmd.AddCommand(mobileinstall.NewDefaultCmd())
	cmd.AddCommand(mod.NewDefaultCmd())
	cmd.AddCommand(print.NewDefaultCmd())
	cmd.AddCommand(printaction.NewDefaultCmd())
	cmd.AddCommand(query.NewDefaultCmd())
	cmd.AddCommand(run.NewDefaultCmd(pluginSystem))
	cmd.AddCommand(sync.NewDefaultCmd())
	cmd.AddCommand(shutdown.NewDefaultCmd())
	cmd.AddCommand(test.NewDefaultCmd(pluginSystem))
	cmd.AddCommand(version.NewDefaultCmd())
	cmd.AddCommand(outputs.NewDefaultCmd())
	cmd.SetHelpCommand(help.NewCmd())

	if buildinfo.Current().OpenSource {
		// Aspect CLI OSS command configurations
		cmd.AddCommand(lint.NewDefaultCmd(pluginSystem))
		cmd.AddCommand(license.NewDefaultCmd())
		cmd.AddCommand(configure.NewDefaultCmd())
	}

	return cmd
}
