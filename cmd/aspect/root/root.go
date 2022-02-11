/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package root

import (
	"os"

	"github.com/fatih/color"
	"github.com/mattn/go-isatty"
	"github.com/spf13/cobra"

	"aspect.build/cli/cmd/aspect/aquery"
	"aspect.build/cli/cmd/aspect/build"
	"aspect.build/cli/cmd/aspect/clean"
	"aspect.build/cli/cmd/aspect/cquery"
	"aspect.build/cli/cmd/aspect/docs"
	"aspect.build/cli/cmd/aspect/info"
	"aspect.build/cli/cmd/aspect/query"
	"aspect.build/cli/cmd/aspect/run"
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

func NewDefaultRootCmd(pluginSystem system.PluginSystem) (*cobra.Command, error) {
	defaultInteractive := isatty.IsTerminal(os.Stdout.Fd()) || isatty.IsCygwinTerminal(os.Stdout.Fd())
	return NewRootCmd(ioutils.DefaultStreams, pluginSystem, defaultInteractive)
}

func NewRootCmd(
	streams ioutils.Streams,
	pluginSystem system.PluginSystem,
	defaultInteractive bool,
) (*cobra.Command, error) {
	cmd := &cobra.Command{
		Use:           "aspect",
		Short:         "Aspect.build bazel wrapper",
		SilenceUsage:  true,
		SilenceErrors: true,
		Long:          boldCyan.Sprintf(`Aspect CLI`) + ` is a better frontend for running bazel`,
		// Suppress timestamps in generated Markdown, for determinism
		DisableAutoGenTag: true,
	}

	flags.AddGlobalFlags(cmd, defaultInteractive)

	// ### Child commands
	// IMPORTANT: when adding a new command, also update the _DOCS list in /docs/BUILD.bazel
	cmd.AddCommand(build.NewDefaultBuildCmd(pluginSystem))
	cmd.AddCommand(clean.NewDefaultCleanCmd())
	cmd.AddCommand(docs.NewDefaultDocsCmd())
	cmd.AddCommand(info.NewDefaultInfoCmd())
	cmd.AddCommand(aquery.NewDefaultAQueryCmd())
	cmd.AddCommand(cquery.NewDefaultCQueryCmd())
	cmd.AddCommand(query.NewDefaultQueryCmd())
	cmd.AddCommand(run.NewDefaultRunCmd(pluginSystem))
	cmd.AddCommand(test.NewDefaultTestCmd(pluginSystem))
	cmd.AddCommand(version.NewDefaultVersionCmd())

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

	// Run this command after all bazel verbs have been added to "cmd".
	err := flags.AddBazelFlags(cmd)

	return cmd, err
}
