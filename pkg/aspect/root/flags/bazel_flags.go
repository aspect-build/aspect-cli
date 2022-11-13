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

package flags

import (
	"fmt"
	"strings"

	"github.com/spf13/cobra"

	"aspect.build/cli/bazel/flags"
	"aspect.build/cli/pkg/bazel"
)

var (
	// Bazel flags specified here will be shown when running "aspect help".
	// By default bazel flags are hidden.
	documentedBazelFlags = []string{
		"keep_going",
		"expunge",
		"expunge_async",
		"show_make_env",
	}
)

// AddBazelFlags will process the configured cobra commands and add bazel
// flags to those commands.
func AddBazelFlags(cmd *cobra.Command) error {
	subCommands := make(map[string]*cobra.Command)

	for _, subCmd := range cmd.Commands() {
		subCmdName := strings.SplitN(subCmd.Use, " ", 2)[0]
		subCommands[subCmdName] = subCmd
	}

	bzl, err := bazel.FindFromWd()
	if err != nil {
		// We cannot run Bazel, but this just means we have no flags to add.
		// This will be the case when running aspect help from outside a workspace, for example.
		// If Bazel is really needed for the current command, an error will be handled somewhere else.
		return nil
	}
	bzlFlags, err := bzl.Flags()
	if err != nil {
		return fmt.Errorf("unable to determine available bazel flags: %w", err)
	}

	for flagName := range bzlFlags {
		flag := bzlFlags[flagName]
		flagAbbreviation := flag.GetAbbreviation()
		flagDoc := flag.GetDocumentation()

		for _, command := range flag.Commands {
			if command == "startup" {
				if flag.GetHasNegativeFlag() {
					RegisterNoableBool(cmd.PersistentFlags(), flagName, false, flagDoc)
					markFlagAsHidden(cmd, flagName)
					markFlagAsHidden(cmd, flags.NoName(flagName))
				} else if flag.GetAllowsMultiple() {
					var key = MultiString{}
					cmd.PersistentFlags().VarP(&key, flagName, flagAbbreviation, flagDoc)
					markFlagAsHidden(cmd, flagName)
				} else {
					cmd.PersistentFlags().StringP(flagName, flagAbbreviation, "", flagDoc)
					markFlagAsHidden(cmd, flagName)
				}

			}
			if subcommand, ok := subCommands[command]; ok {
				subcommand.DisableFlagParsing = true // only want to disable flag parsing on actual bazel verbs
				if flag.GetHasNegativeFlag() {
					RegisterNoableBoolP(subcommand.Flags(), flagName, flagAbbreviation, false, flagDoc)
					markFlagAsHidden(subcommand, flagName)
					markFlagAsHidden(subcommand, flags.NoName(flagName))
				} else if flag.GetAllowsMultiple() {
					var key = MultiString{}
					subcommand.Flags().VarP(&key, flagName, flagAbbreviation, flagDoc)
					markFlagAsHidden(subcommand, flagName)
				} else {
					subcommand.Flags().StringP(flagName, flagAbbreviation, "", flagDoc)
					markFlagAsHidden(subcommand, flagName)
				}
			}
		}
	}

	return nil
}

func markFlagAsHidden(cmd *cobra.Command, flag string) {
	for _, documentedFlag := range documentedBazelFlags {
		if documentedFlag == flag {
			return
		}
	}

	cmd.Flags().MarkHidden(flag)
	cmd.PersistentFlags().MarkHidden(flag)
}
