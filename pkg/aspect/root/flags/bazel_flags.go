/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package flags

import (
	"fmt"
	"strings"

	"github.com/spf13/cobra"

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

// MultiString is the golang implementation of bazel multi-string arguments that satisfies
// Value from cobra's Flags().Var functions.
type MultiString struct {
	value []string
}

// Set satisfies Value from cobra's Flags().Var functions.
func (s *MultiString) Set(value string) error {
	s.value = append(s.value, value)
	return nil
}

// Type satisfies Value from cobra's Flags().Var functions.
func (s *MultiString) Type() string {
	return "multiString"
}

// String satisfies Value from cobra's Flags().Var functions.
func (s *MultiString) String() string {
	return fmt.Sprintf("[ %s ]", strings.Join(s.value, ", "))
}

// First satisfies Value from cobra's Flags().Var functions
func (s *MultiString) First() string {
	return (s.value)[0]
}

// AddBazelFlags will process the configured cobra commands and add bazel
// flags to those commands.
func AddBazelFlags(cmd *cobra.Command) error {
	subCommands := make(map[string]*cobra.Command)

	for _, command := range cmd.Commands() {
		subCommands[command.Use] = command
	}

	bzl := bazel.New()
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
					cmd.PersistentFlags().Bool(flagName, false, flagDoc)
					cmd.PersistentFlags().Bool("no"+flagName, false, flagDoc)
					markFlagAsHidden(cmd, flagName)
					markFlagAsHidden(cmd, "no"+flagName)
				} else if flag.GetAllowsMultiple() {
					var key = MultiString{value: []string{}}
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
					subcommand.Flags().BoolP(flagName, flagAbbreviation, false, flagDoc)
					subcommand.Flags().Bool("no"+flagName, false, flagDoc)
					markFlagAsHidden(subcommand, flagName)
					markFlagAsHidden(subcommand, "no"+flagName)
				} else if flag.GetAllowsMultiple() {
					var key = MultiString{value: []string{}}
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
