/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package flags

import (
	"fmt"
	"strings"

	"aspect.build/cli/pkg/bazel"
	"github.com/spf13/cobra"
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
	if bzlFlags, err := bzl.Flags(); err != nil {
		return fmt.Errorf("unable to determine available bazel flags: %w", err)
	} else {
		for flag := range bzlFlags {
			for _, command := range bzlFlags[flag].Commands {
				if subcommand, ok := subCommands[command]; ok {
					if bzlFlags[flag].GetHasNegativeFlag() {
						subcommand.Flags().BoolP(flag, bzlFlags[flag].GetAbbreviation(), false, bzlFlags[flag].GetDocumentation())
						subcommand.Flags().Bool("no"+flag, false, bzlFlags[flag].GetDocumentation())
						markFlagAsHidden(subcommand, flag)
						markFlagAsHidden(subcommand, "no"+flag)
					} else if bzlFlags[flag].GetAllowsMultiple() {
						var key = MultiString{value: &[]string{}}
						subcommand.Flags().VarP(&key, flag, bzlFlags[flag].GetAbbreviation(), bzlFlags[flag].GetDocumentation())
						markFlagAsHidden(subcommand, flag)
					} else {
						subcommand.Flags().StringP(flag, bzlFlags[flag].GetAbbreviation(), "", bzlFlags[flag].GetDocumentation())
						markFlagAsHidden(subcommand, flag)
					}
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
