/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package flags

import (
	"fmt"
	"strings"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/bazel"
)

var (
	exposedBazelFlags = []string{
		"keep_going",
		"expunge",
		"expunge_async",
		"show_make_env",
	}
)

type MultiString struct {
	value *[]string
}

func (s *MultiString) Set(value string) error {
	*s.value = append(*s.value, value)
	return nil
}

func (s *MultiString) Type() string {
	return "multiString"
}

func (s *MultiString) String() string {
	return fmt.Sprintf("[ %s ]", strings.Join(*s.value, ", "))
}

func (s *MultiString) First() string {
	return (*s.value)[0]
}

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
			if subcommand, ok := subCommands[command]; ok {
				if flag.GetHasNegativeFlag() {
					subcommand.Flags().BoolP(flagName, flagAbbreviation, false, flagDoc)
					subcommand.Flags().Bool("no"+flagName, false, flagDoc)
					markFlagAsHidden(subcommand, flagName)
					markFlagAsHidden(subcommand, "no"+flagName)
				} else if flag.GetAllowsMultiple() {
					var key = MultiString{value: &[]string{}}
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
	for _, exposedFlag := range exposedBazelFlags {
		if exposedFlag == flag {
			return
		}
	}

	cmd.Flags().MarkHidden(flag)
}
