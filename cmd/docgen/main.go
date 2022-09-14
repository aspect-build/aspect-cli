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

	"github.com/spf13/cobra"
	"github.com/spf13/cobra/doc"

	"aspect.build/cli/cmd/aspect/root"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system"
)

func main() {
	cmd := &cobra.Command{Use: "docgen"}

	pluginSystem := system.NewPluginSystem()
	if err := pluginSystem.Configure(ioutils.DefaultStreams); err != nil {
		log.Fatal(err)
	}
	defer pluginSystem.TearDown()

	aspectRootCmd := root.NewDefaultRootCmd(pluginSystem)

	// Run this command after all bazel verbs have been added to "cmd".
	if err := flags.AddBazelFlags(cmd); err != nil {
		log.Fatal(err)
	}

	if err := pluginSystem.RegisterCustomCommands(cmd); err != nil {
		log.Fatal(err)
	}

	cmd.AddCommand(NewBzlCommandListCmd(aspectRootCmd))
	cmd.AddCommand(NewGenMarkdownCmd(aspectRootCmd))

	if err := cmd.Execute(); err != nil {
		log.Fatal(err)
	}
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
					fmt.Printf("    %q,\n", cmd.Use)
				}
			}
			fmt.Println("]")
			return nil
		},
	}
	return cmd
}

func NewGenMarkdownCmd(aspectRootCmd *cobra.Command) *cobra.Command {
	var outputDir string

	cmd := &cobra.Command{
		Use:   "gen-markdown",
		Short: "Generates the markdown documentation.",
		RunE: func(_ *cobra.Command, _ []string) error {
			return doc.GenMarkdownTree(aspectRootCmd, outputDir)
		},
	}

	outputDirFlag := "output-dir"
	cmd.PersistentFlags().StringVar(&outputDir, outputDirFlag, "", "The path to the output directory.")
	cmd.MarkPersistentFlagRequired(outputDirFlag)

	return cmd
}
