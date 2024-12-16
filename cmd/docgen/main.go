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

package main

import (
	"fmt"
	"log"
	"os"
	"path"
	"path/filepath"
	"strings"

	"github.com/spf13/cobra"
	"github.com/spf13/cobra/doc"

	"github.com/aspect-build/aspect-cli/cmd/aspect/root"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/aspect-build/aspect-cli/pkg/plugin/system"
)

func main() {
	bzl := bazel.WorkspaceFromWd

	if err := bzl.InitializeBazelFlags(); err != nil {
		log.Fatal(err)
	}

	args, startupFlags, err := bazel.InitializeStartupFlags(os.Args[1:])

	if err != nil {
		log.Fatal(err)
	}

	if err = command(bzl, args, startupFlags); err != nil {
		log.Fatal(err)
	}
}

func command(bzl bazel.Bazel, args []string, startupFlags []string) error {
	cmd := &cobra.Command{Use: "docgen"}

	pluginSystem := system.NewPluginSystem()

	if !root.CheckAspectDisablePluginsFlag(args) {
		if err := pluginSystem.Configure(ioutils.DefaultStreams, nil); err != nil {
			return err
		}
	}

	defer pluginSystem.TearDown()

	aspectRootCmd := root.NewDefaultCmd(pluginSystem)

	// Run this command after all bazel verbs have been added to "cmd".
	if err := bzl.AddBazelFlags(cmd); err != nil {
		return err
	}

	if err := pluginSystem.RegisterCustomCommands(cmd, startupFlags); err != nil {
		return err
	}

	cmd.AddCommand(NewBzlCommandListCmd(aspectRootCmd))
	cmd.AddCommand(NewGenMarkdownCmd(aspectRootCmd))

	if err := cmd.Execute(); err != nil {
		return err
	}

	return nil
}

func NewBzlCommandListCmd(aspectRootCmd *cobra.Command) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "bzl-command-list",
		Short: "Prints a .bzl file with the top-level list of commands of the aspect CLI.",
		Long: "This command is used to produce the .bzl file with the list of top-level commands" +
			"used to automatically generate markdown documentation for this repository.",
		RunE: func(_ *cobra.Command, _ []string) (exitErr error) {
			fmt.Println(`"""Generated file - do NOT edit!`)
			fmt.Println("This module contains the list of top-level commands from the aspect CLI.")
			fmt.Println(`"""`)
			fmt.Println("COMMAND_LIST = [")
			cmds := aspectRootCmd.Commands()
			for _, cmd := range cmds {
				if cmd.IsAvailableCommand() {
					cmdName := strings.SplitN(cmd.Use, " ", 2)[0]
					fmt.Printf("    %q,\n", cmdName)
				}
			}
			fmt.Println("]")
			return nil
		},
	}
	return cmd
}

const fmTemplate = `---
sidebar_label: "%s"
---
`

func NewGenMarkdownCmd(aspectRootCmd *cobra.Command) *cobra.Command {
	var outputDir string

	// Customized output, see
	// https://github.com/spf13/cobra/blob/main/doc/md_docs.md#customize-the-output
	filePrepender := func(filename string) string {
		name := filepath.Base(filename)
		base := strings.TrimSuffix(name, path.Ext(name))
		return fmt.Sprintf(fmTemplate, strings.TrimPrefix(base, "aspect_"))
	}
	linkHandler := func(name string) string {
		return name
	}

	cmd := &cobra.Command{
		Use:   "gen-markdown",
		Short: "Generates the markdown documentation.",
		RunE: func(_ *cobra.Command, _ []string) error {
			return doc.GenMarkdownTreeCustom(aspectRootCmd, outputDir, filePrepender, linkHandler)
		},
	}

	outputDirFlag := "output-dir"
	cmd.PersistentFlags().StringVar(&outputDir, outputDirFlag, "", "The path to the output directory.")
	cmd.MarkPersistentFlagRequired(outputDirFlag)

	return cmd
}
