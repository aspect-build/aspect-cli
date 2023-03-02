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

	"aspect.build/cli/bazel/flags"
	rootFlags "aspect.build/cli/pkg/aspect/root/flags"
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
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

	// Bazel flags that expand to other flags. These are boolean flags that are not no-able. Currently
	// there is no way to detect these so we have to keep list up-to-date manually with the union of
	// these flags across all Bazel versions we support.
	// These were gathered by searching https://bazel.build/reference/command-line-reference for "Expands to:"
	expandoFlags = map[string]struct{}{
		"debug_app":                     {},
		"experimental_persistent_javac": {},
		"experimental_spawn_scheduler":  {},
		"expunge_async":                 {},
		"host_jvm_debug":                {},
		"java_debug":                    {},
		"long":                          {},
		"noincompatible_genquery_use_graphless_query": {},
		"noorder_results":                                 {},
		"null":                                            {},
		"order_results":                                   {},
		"persistent_android_dex_desugar":                  {},
		"persistent_android_resource_processor":           {},
		"persistent_multiplex_android_dex_desugar":        {},
		"persistent_multiplex_android_resource_processor": {},
		"persistent_multiplex_android_tools":              {},
		"remote_download_minimal":                         {},
		"remote_download_toplevel":                        {},
		"short":                                           {},
		"start_app":                                       {},
	}

	bazelFlagSets = map[string]*pflag.FlagSet{}
)

func addFlagToFlagSet(flag *flags.FlagInfo, flagSet *pflag.FlagSet) {
	flagName := flag.GetName()
	flagAbbreviation := flag.GetAbbreviation()
	flagDoc := flag.GetDocumentation()

	if flag.GetHasNegativeFlag() {
		rootFlags.RegisterNoableBoolP(flagSet, flagName, flagAbbreviation, false, flagDoc)
	} else if flag.GetAllowsMultiple() {
		value := rootFlags.MultiString{}
		flagSet.VarP(&value, flagName, flagAbbreviation, flagDoc)
	} else {
		if isExpando(flagName) {
			flagSet.BoolP(flagName, flagAbbreviation, false, flagDoc)
		} else {
			flagSet.StringP(flagName, flagAbbreviation, "", flagDoc)
		}
	}

}

// InitializeBazelFlags will create FlagSets for each bazel command (including
// the special startup "command" set). These are used later by ParseOutBazelFlags
// which is called by InitializeStartUp flags and some special-case commands
// such as query, cquery and aquery.
func (b *bazel) InitializeBazelFlags() error {
	bzlFlags, err := b.Flags()
	if err != nil {
		return err
	}

	for flagName := range bzlFlags {
		flag := bzlFlags[flagName]

		for _, command := range flag.Commands {
			flagSet := bazelFlagSets[command]
			if flagSet == nil {
				flagSet = pflag.NewFlagSet(command, pflag.ContinueOnError)
				bazelFlagSets[command] = flagSet
			}
			addFlagToFlagSet(flag, flagSet)
		}
	}
	return nil
}

// AddBazelFlags will process the configured cobra commands and add bazel
// flags to those commands.
func (b *bazel) AddBazelFlags(cmd *cobra.Command) error {
	subCommands := make(map[string]*cobra.Command)

	for _, subCmd := range cmd.Commands() {
		subCmdName := strings.SplitN(subCmd.Use, " ", 2)[0]
		subCommands[subCmdName] = subCmd
	}

	bzlFlags, err := b.Flags()
	if err != nil {
		return err
	}

	for flagName := range bzlFlags {
		flag := bzlFlags[flagName]
		documented := isDocumented(flagName)

		for _, command := range flag.Commands {
			if subcommand, ok := subCommands[command]; ok {
				subcommand.DisableFlagParsing = true // only want to disable flag parsing on actual bazel verbs
				subcommand.FParseErrWhitelist.UnknownFlags = true
				if documented {
					addFlagToFlagSet(flag, subcommand.Flags())
				}
			}
		}
	}

	return nil
}

// Separates bazel flags from a list of arguments for the given bazel command.
// Returns the non-flag arguments & flag arguments as separate lists
func ParseOutBazelFlags(command string, args []string) ([]string, []string, error) {
	bazelFlags := bazelFlagSets[command]
	if bazelFlags == nil {
		for _, s := range args {
			if len(s) > 1 && s[1] == '-' {
				// there are args to parse but we don't know the flags for this bazel command
				return nil, nil, fmt.Errorf("%v not a recognized bazel command", command)
			}
		}
		// we don't know the flags for this bazel command, but there are no flags to parse; this is
		// likely a unit test
		return args, []string{}, nil
	}

	flags := make([]string, 0, len(args))
	nonFlags := make([]string, 0, len(args))

	for len(args) > 0 {
		s := args[0]
		args = args[1:]
		if len(s) == 0 || s[0] != '-' || len(s) == 1 {
			nonFlags = append(nonFlags, s)
			if command == "startup" {
				// special case startup flags which must come before the first command
				nonFlags = append(nonFlags, args...)
				break
			}
			continue
		}

		if s[1] == '-' {
			if len(s) == 2 { // "--" terminates the flags
				nonFlags = append(nonFlags, s)
				nonFlags = append(nonFlags, args...)
				break
			}
			// long arg
			name := s[2:]
			if len(name) == 0 || name[0] == '-' || name[0] == '=' {
				return nil, nil, fmt.Errorf("bad flag syntax: %s", s)
			}
			split := strings.SplitN(name, "=", 2)
			name = split[0]
			flag := bazelFlags.Lookup(name)
			if name == "version" || name == "help" {
				// --version and --help special cases
				nonFlags = append(nonFlags, s)
			} else if strings.HasPrefix(name, "aspect:") {
				// --aspect:* special case
				nonFlags = append(nonFlags, s)
			} else if flag == nil {
				return nil, nil, fmt.Errorf("unknown %s flag: --%s", command, name)
			} else if len(split) == 2 {
				// '--flag=arg'
				flags = append(flags, s)
			} else if flag.NoOptDefVal != "" {
				// '--flag' (arg was optional)
				flags = append(flags, s)
			} else if len(args) > 0 {
				// '--flag arg'
				flags = append(flags, s)
				flags = append(flags, args[0])
				args = args[1:]
			} else {
				// '--flag' (arg was required)
				return nil, nil, fmt.Errorf("flag needs an argument: %s", s)
			}
		} else {
			// short arg
			if s == "-v" || s == "-h" {
				// -v and -h special cases
				nonFlags = append(nonFlags, s)
			}
			flags = append(flags, s)
		}
	}

	return nonFlags, flags, nil
}

func isExpando(flag string) bool {
	_, ok := expandoFlags[flag]
	return ok
}

func isDocumented(flag string) bool {
	for _, documentedFlag := range documentedBazelFlags {
		if documentedFlag == flag {
			return true
		}
	}
	return false
}
