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

package bazel

import (
	"fmt"
	"strings"

	"aspect.build/cli/pkg/aspect/root/flags"
	"github.com/spf13/cobra"
)

var (
	// Bazel flags specified here will be shown when running "aspect help".
	// By default bazel flags are hidden.
	documentedBazelFlags = []string{
		"keep_going",
		"expunge",
		"expunge_async",
		"show_make_env",
		"gnu_format",
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

	bzl, err := FindFromWd()
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
		documented := isDocumented(flagName)

		for _, command := range flag.Commands {
			if subcommand, ok := subCommands[command]; ok {
				subcommand.DisableFlagParsing = true // only want to disable flag parsing on actual bazel verbs
				subcommand.FParseErrWhitelist.UnknownFlags = true
				if !documented {
					continue
				}
				if flag.GetHasNegativeFlag() {
					flags.RegisterNoableBoolP(subcommand.Flags(), flagName, flagAbbreviation, false, flagDoc)
				} else if flag.GetAllowsMultiple() {
					var key = flags.MultiString{}
					subcommand.Flags().VarP(&key, flagName, flagAbbreviation, flagDoc)
				} else {
					subcommand.Flags().StringP(flagName, flagAbbreviation, "", flagDoc)
				}
			}
		}
	}

	return nil
}

func isDocumented(flag string) bool {
	for _, documentedFlag := range documentedBazelFlags {
		if documentedFlag == flag {
			return true
		}
	}
	return false
}
