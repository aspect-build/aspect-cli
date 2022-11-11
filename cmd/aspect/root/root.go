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
	"aspect.build/cli/cmd/aspect/configure"
	"aspect.build/cli/cmd/aspect/cquery"
	"aspect.build/cli/cmd/aspect/docs"
	"aspect.build/cli/cmd/aspect/dump"
	"aspect.build/cli/cmd/aspect/fetch"
	"aspect.build/cli/cmd/aspect/info"
	"aspect.build/cli/cmd/aspect/mobileinstall"
	"aspect.build/cli/cmd/aspect/modquery"
	"aspect.build/cli/cmd/aspect/outputs"
	"aspect.build/cli/cmd/aspect/print"
	"aspect.build/cli/cmd/aspect/printaction"
	"aspect.build/cli/cmd/aspect/query"
	"aspect.build/cli/cmd/aspect/run"
	"aspect.build/cli/cmd/aspect/shutdown"
	"aspect.build/cli/cmd/aspect/support"
	"aspect.build/cli/cmd/aspect/sync"
	"aspect.build/cli/cmd/aspect/test"
	"aspect.build/cli/cmd/aspect/version"
	"aspect.build/cli/docs/help/topics"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system"
)

var (
	boldCyan = color.New(color.FgCyan, color.Bold)
	faint    = color.New(color.Faint)
)

func NewProRootCmd(pluginSystem system.PluginSystem) *cobra.Command {
	defaultInteractive := isatty.IsTerminal(os.Stdin.Fd()) || isatty.IsCygwinTerminal(os.Stdin.Fd())
	return NewRootCmd(ioutils.DefaultStreams, pluginSystem, defaultInteractive, true)
}

func NewDefaultRootCmd(pluginSystem system.PluginSystem) *cobra.Command {
	defaultInteractive := isatty.IsTerminal(os.Stdin.Fd()) || isatty.IsCygwinTerminal(os.Stdin.Fd())
	return NewRootCmd(ioutils.DefaultStreams, pluginSystem, defaultInteractive, false)
}

func NewRootCmd(
	streams ioutils.Streams,
	pluginSystem system.PluginSystem,
	defaultInteractive bool,
	pro bool,
) *cobra.Command {
	productName := "Aspect CLI"
	if pro {
		productName = "Aspect CLI Pro"
	}

	cmd := &cobra.Command{
		Use:           "aspect",
		Short:         productName,
		SilenceUsage:  true,
		SilenceErrors: true,
		Long:          boldCyan.Sprintf(productName) + ` is a better frontend for running bazel`,
		// Suppress timestamps in generated Markdown, for determinism
		DisableAutoGenTag: true,
		Version:           buildinfo.Current().Version(),
	}
	cmd.SetVersionTemplate(`{{with .Name}}{{printf "%s " .}}{{end}}{{printf "%s" .Version}}
`)

	flags.AddGlobalFlags(cmd, defaultInteractive)
	cmd.AddGroup(&cobra.Group{ID: "common", Title: "Common Bazel Commands:"})
	cmd.AddGroup(&cobra.Group{ID: "aspect", Title: "Commands only in Aspect CLI:"})
	cmd.AddGroup(&cobra.Group{ID: "plugin", Title: "Custom Commands from Plugins:"})
	cmd.AddGroup(&cobra.Group{ID: "built-in", Title: "Other Bazel Built-in Commands:"})

	// ### Child commands
	// IMPORTANT: when adding a new command, also update the COMMAND_LIST list in /docs/command_list.bzl
	cmd.AddCommand(analyzeprofile.NewDefaultAnalyzeProfileCmd())
	cmd.AddCommand(aquery.NewDefaultAQueryCmd())
	cmd.AddCommand(build.NewDefaultBuildCmd(pluginSystem))
	cmd.AddCommand(canonicalizeflags.NewDefaultCanonicalizeFlagsCmd())
	cmd.AddCommand(clean.NewDefaultCleanCmd())
	cmd.AddCommand(cquery.NewDefaultCQueryCmd())
	cmd.AddCommand(dump.NewDefaultDumpCmd())
	cmd.AddCommand(fetch.NewDefaultFetchCmd())
	cmd.AddCommand(docs.NewDefaultDocsCmd())
	cmd.AddCommand(info.NewDefaultInfoCmd())
	// license
	cmd.AddCommand(mobileinstall.NewDefaultMobileInstallCmd())
	cmd.AddCommand(modquery.NewDefaultModQueryCmd())
	cmd.AddCommand(print.NewDefaultPrintCmd())
	cmd.AddCommand(printaction.NewDefaultPrintActionCmd())
	cmd.AddCommand(query.NewDefaultQueryCmd())
	cmd.AddCommand(run.NewDefaultRunCmd(pluginSystem))
	cmd.AddCommand(sync.NewDefaultSyncCmd())
	cmd.AddCommand(shutdown.NewDefaultShutdownCmd())
	cmd.AddCommand(test.NewDefaultTestCmd(pluginSystem))
	cmd.AddCommand(version.NewDefaultVersionCmd())
	cmd.AddCommand(outputs.NewDefaultOutputsCmd())
	if !pro {
		// Pro command stubs
		cmd.AddCommand(configure.NewDefaultConfigureCmd())
		cmd.AddCommand(support.NewDefaultSupportCmd())
	}

	// ### "Additional help topic commands" which are not runnable
	// https://pkg.go.dev/github.com/spf13/cobra#Command.IsAdditionalHelpTopicCommand
	cmd.AddCommand(&cobra.Command{
		Use:   "target-syntax",
		Short: "Explains the syntax for specifying targets.",
		Long:  topics.MustAssetString("target-syntax.md"),
	})
	cmd.AddCommand(&cobra.Command{
		Use:   "info-keys",
		Short: "Displays a list of keys used by the info command.",
		Long:  topics.MustAssetString("info-keys.md"),
	})
	cmd.AddCommand(&cobra.Command{
		Use:   "tags",
		Short: "Conventions for tags which are special.",
		Long:  topics.MustAssetString("tags.md"),
	})
	cmd.AddCommand(&cobra.Command{
		Use:   "plugins",
		Short: "How to extend aspect with a custom plugin.",
		Long:  topics.MustAssetString("plugins.md"),
	})
	return cmd
}
