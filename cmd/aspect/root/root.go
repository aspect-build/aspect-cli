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

	"github.com/aspect-build/aspect-cli/buildinfo"
	"github.com/aspect-build/aspect-cli/cmd/aspect/analyzeprofile"
	"github.com/aspect-build/aspect-cli/cmd/aspect/aquery"
	"github.com/aspect-build/aspect-cli/cmd/aspect/build"
	"github.com/aspect-build/aspect-cli/cmd/aspect/canonicalizeflags"
	"github.com/aspect-build/aspect-cli/cmd/aspect/clean"
	"github.com/aspect-build/aspect-cli/cmd/aspect/config"
	"github.com/aspect-build/aspect-cli/cmd/aspect/configure"
	"github.com/aspect-build/aspect-cli/cmd/aspect/coverage"
	"github.com/aspect-build/aspect-cli/cmd/aspect/cquery"
	"github.com/aspect-build/aspect-cli/cmd/aspect/docs"
	"github.com/aspect-build/aspect-cli/cmd/aspect/dump"
	"github.com/aspect-build/aspect-cli/cmd/aspect/fetch"
	"github.com/aspect-build/aspect-cli/cmd/aspect/help"
	"github.com/aspect-build/aspect-cli/cmd/aspect/info"
	init_ "github.com/aspect-build/aspect-cli/cmd/aspect/init"
	"github.com/aspect-build/aspect-cli/cmd/aspect/license"
	"github.com/aspect-build/aspect-cli/cmd/aspect/lint"
	"github.com/aspect-build/aspect-cli/cmd/aspect/mobileinstall"
	"github.com/aspect-build/aspect-cli/cmd/aspect/mod"
	"github.com/aspect-build/aspect-cli/cmd/aspect/outputs"
	"github.com/aspect-build/aspect-cli/cmd/aspect/print"
	"github.com/aspect-build/aspect-cli/cmd/aspect/printaction"
	"github.com/aspect-build/aspect-cli/cmd/aspect/query"
	"github.com/aspect-build/aspect-cli/cmd/aspect/run"
	"github.com/aspect-build/aspect-cli/cmd/aspect/shutdown"
	"github.com/aspect-build/aspect-cli/cmd/aspect/sync"
	"github.com/aspect-build/aspect-cli/cmd/aspect/test"
	vendor "github.com/aspect-build/aspect-cli/cmd/aspect/vend"
	"github.com/aspect-build/aspect-cli/cmd/aspect/version"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/aspect-build/aspect-cli/pkg/plugin/system"
)

var (
	boldCyan = color.New(color.FgCyan, color.Bold)
	faint    = color.New(color.Faint)
)

func NewDefaultCmd(pluginSystem system.PluginSystem) *cobra.Command {
	defaultInteractive := isatty.IsTerminal(os.Stdin.Fd()) || isatty.IsCygwinTerminal(os.Stdin.Fd())
	// Some CI systems attach a TTY, but we shouldn't prompt there
	if _, ok := os.LookupEnv("CI"); ok {
		defaultInteractive = false
	}
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
	cmd.AddCommand(vendor.NewDefaultCmd())
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
