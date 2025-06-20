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
	"bytes"
	"errors"
	"fmt"
	"os"
	"path"
	"path/filepath"
	"regexp"
	"strings"

	"github.com/aspect-build/aspect-cli/bazel/flags"
	rootFlags "github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/bazelbuild/buildtools/edit"
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
)

var (
	// Bazel flags specified here will be shown when running "aspect help".
	// All other flags are hidden by default so that "help" is not overwhelming for users.
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
		"remote_download_all":                             {},
		"remote_download_minimal":                         {},
		"remote_download_toplevel":                        {},
		"short":                                           {},
		"start_app":                                       {},
	}

	// List of all commands with label as inputs
	commandsWithLabelInput = map[string]struct{}{
		"aquery":         {},
		"build":          {},
		"coverage":       {},
		"cquery":         {},
		"fetch":          {},
		"lint":           {},
		"mobile-install": {},
		"outputs":        {},
		"print-action":   {},
		"query":          {},
		"run":            {},
		"test":           {},
	}

	bazelFlagSets = map[string]*pflag.FlagSet{}
)

func addFlagToFlagSet(flag *flags.FlagInfo, flagSet *pflag.FlagSet, hidden bool) {
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
	if hidden {
		flagSet.MarkHidden(flagName)
	}
}

// InitializeBazelFlags will create FlagSets for each bazel command (including
// the special startup "command" set). These are used later by SeparateBazelFlags
// which is called by InitializeStartUp flags and some special-case commands
// such as query, cquery and aquery.
func (b *bazel) InitializeBazelFlags() error {
	flags, err := b.Flags()
	if err != nil {
		return err
	}

	for _, flag := range flags {
		for _, command := range flag.Commands {
			flagSet := bazelFlagSets[command]
			if flagSet == nil {
				flagSet = pflag.NewFlagSet(command, pflag.ContinueOnError)
				bazelFlagSets[command] = flagSet
			}
			addFlagToFlagSet(flag, flagSet, true)
		}
	}
	return nil
}

// Returns true if a flag is part of a bazel command
func (b *bazel) IsBazelFlag(command string, flag string) (bool, error) {
	flags, err := b.Flags()
	if err != nil {
		return false, err
	}

	for flagName, f := range flags {
		for _, cmd := range f.Commands {
			if cmd == command && flagName == flag {
				return true, nil
			}
		}
	}
	return false, nil
}

// AddBazelFlags will process the configured cobra commands and add bazel
// flags to those commands.
func (b *bazel) AddBazelFlags(cmd *cobra.Command) error {
	completionCommands := make(map[string]*cobra.Command)

	commands := make(map[string]*cobra.Command)
	for _, c := range cmd.Commands() {
		name := strings.SplitN(c.Use, " ", 2)[0]
		commands[name] = c
	}

	flags, err := b.Flags()
	if err != nil {
		return err
	}

	for flagName, flag := range flags {
		documented := isDocumented(flagName)

		for _, commandName := range flag.Commands {
			commandNames := []string{commandName}
			if commandName == "aquery" {
				// outputs call aquery under the hood and accept all aquery flags
				commandNames = append(commandNames, "outputs")
			}
			if commandName == "build" {
				// lint calls build under the hood and accepts all build flags
				commandNames = append(commandNames, "lint")
			}
			for _, n := range commandNames {
				if c, ok := commands[n]; ok {
					c.DisableFlagParsing = true // only want to disable flag parsing on commands that call out to bazel
					addFlagToFlagSet(flag, c.Flags(), !documented)

					// Collect all the commands that have at least one flag defined for completion.
					// The subset of commands with label inputs (commandsWithLabelInput) for
					// completion all have at least one flag defined so are captured in this list.
					completionCommands[n] = c
				}
			}
		}
	}

	// Register startup flags to main command. We disable flag parsing such that the cobra completion
	// triggers the ValidArgsFunction of the root command.
	cmd.DisableFlagParsing = true
	cmd.ValidArgsFunction = func(cmd *cobra.Command, args []string, toComplete string) ([]string, cobra.ShellCompDirective) {
		if toComplete == "" {
			return nil, cobra.ShellCompDirectiveDefault
		}
		return listBazelFlags("startup"), cobra.ShellCompDirectiveDefault
	}

	// Register custom ValidArgsFunction to add flag auto-completion for bazel defined flags.
	for n, c := range completionCommands {
		if _, ok := commandsWithLabelInput[n]; ok {
			c.ValidArgsFunction = b.validArgsWithLabelAndPackages(n)
			continue
		}
		c.ValidArgsFunction = b.validArgsWithFlags(n)
	}

	return nil
}

// validArgsWithFlags creates a ValidArgsFunction that completes flags for the given command.
func (b *bazel) validArgsWithFlags(name string) func(*cobra.Command, []string, string) ([]string, cobra.ShellCompDirective) {
	return func(_ *cobra.Command, _ []string, toComplete string) ([]string, cobra.ShellCompDirective) {
		return listBazelFlags(name), cobra.ShellCompDirectiveDefault
	}
}

// validArgsWithLabelAndPackages creates a ValidArgsFunction that completes both
// flags and labels for the given command.
func (b *bazel) validArgsWithLabelAndPackages(name string) func(*cobra.Command, []string, string) ([]string, cobra.ShellCompDirective) {
	return func(cmd *cobra.Command, _ []string, toComplete string) ([]string, cobra.ShellCompDirective) {
		// Complete flags
		if strings.HasPrefix(toComplete, "-") {
			return listBazelFlags(name), cobra.ShellCompDirectiveDefault
		}

		// Complete labels
		var results []string
		workspaceRegex := regexp.MustCompile(`^@@?\/?`)
		workspaceLabel := toComplete == "@" || toComplete == "@@" || workspaceRegex.MatchString(toComplete)
		workspacePrefix := ""
		if workspaceLabel {
			workspacePrefix = "@"
			if strings.HasPrefix(toComplete, "@@") {
				workspacePrefix = "@@"
			}
			toComplete = strings.TrimLeft(toComplete, "@")
		}
		labelRegex := regexp.MustCompile(`^\/\/[^\/]+`)
		absLabel := workspaceLabel || toComplete == "/" || toComplete == "//" || labelRegex.MatchString(toComplete)
		searchPkg, _, _ := strings.Cut(toComplete, ":")
		searchPkg = strings.TrimLeft(searchPkg, "/")
		searchPkg = strings.TrimSuffix(searchPkg, "/")
		trailingSlash := strings.HasSuffix(toComplete, "/")
		rootDir := b.workspaceRoot
		workspaceCwd := ""

		// If the completion string is not an absolute label then look for packages and labels
		// relative to the current working directory
		if !absLabel {
			cwd, err := os.Getwd()
			if err != nil {
				return nil, cobra.ShellCompDirectiveError
			}
			rootDir = cwd
			workspaceCwd = strings.TrimSuffix(strings.TrimPrefix(cwd, b.workspaceRoot), "/")
		}

		// Search for labels if there is not a trailing slash on the completion string
		if !trailingSlash {
			targets, _ := listBazelRules(workspaceCwd, searchPkg)
			for _, l := range targets {
				if absLabel {
					l = workspacePrefix + "//" + l
				}
				results = append(results, l)
			}
		}

		// If there is not a trailing slash on the completion string then
		// the search package is the parent package
		if searchPkg != "" && !trailingSlash {
			segments := strings.Split(searchPkg, "/")
			searchPkg = strings.Join(segments[0:len(segments)-1], "/")
		}

		// Search for bazel packages
		packages, _ := b.expandPackageNames(rootDir, searchPkg, true)
		for _, p := range packages {
			if absLabel {
				p = workspacePrefix + "//" + p
			}
			if p == toComplete {
				// if the suggested package matches toComplete then suggest to recurse into
				// the package for sub-packages
				p = p + "/"
			}
			results = append(results, p)
		}

		return results, cobra.ShellCompDirectiveNoSpace
	}
}

// Helper function to check if file exists
func fileExists(f string) bool {
	_, err := os.Stat(f)
	return err == nil
}

func (b *bazel) expandPackageNames(rootDir string, searchPkg string, recurse bool) ([]string, error) {
	pkgDir := filepath.Join(rootDir, searchPkg)

	var results []string

	if searchPkg != "" && (fileExists(filepath.Join(pkgDir, "BUILD")) || fileExists(filepath.Join(pkgDir, "BUILD.bazel"))) {
		results = append(results, searchPkg)
		if !recurse {
			return results, nil
		}
	}

	entries, err := os.ReadDir(pkgDir)
	if err != nil && errors.Is(err, os.ErrNotExist) {
		// Directory does not exist
		return []string{}, nil
	}
	if err != nil {
		return nil, err
	}

	for _, e := range entries {
		// Only directories can be packages.
		if !e.IsDir() {
			continue
		}
		// Skip symlinks (e.g. bazel-bin)
		if e.Type()&os.ModeSymlink != 0 {
			continue
		}
		// Skip .git
		if e.Name() == ".git" {
			continue
		}
		// Recurse into the directory to look for Bazel packages
		recursive, _ := b.expandPackageNames(rootDir, filepath.Join(searchPkg, e.Name()), false)
		results = append(results, recursive...)
	}

	return results, nil
}

func listBazelRules(workspaceCwd string, completionPkg string) ([]string, error) {
	pkg := path.Join(workspaceCwd, completionPkg)

	var stdout bytes.Buffer
	var stderr strings.Builder
	opts := &edit.Options{
		OutWriter: &stdout,
		ErrWriter: &stderr,
		NumIO:     200,
	}
	if ret := edit.Buildozer(opts, []string{"print label", "//" + pkg + ":all"}); ret != 0 {
		return nil, fmt.Errorf("buildozer exit %d: %s", ret, stderr.String())
	}

	var results []string

	rules := strings.Split(strings.TrimSpace(stdout.String()), "\n")

	// Do post-processing on the labels so that results start with completionPkg
	for _, t := range rules {
		if t == "" {
			continue
		}
		if _, label, ok := strings.Cut(t, ":"); ok {
			results = append(results, completionPkg+":"+label)
		} else {
			results = append(results, completionPkg+":"+path.Base(t))
		}
	}

	return results, nil
}

// List all bazel flags for a command
func listBazelFlags(command string) []string {
	flags, ok := bazelFlagSets[command]
	if !ok {
		return nil
	}
	var result []string
	flags.VisitAll(func(outFile *pflag.Flag) {
		result = append(result, "--"+outFile.Name)
	})
	return result
}

// Separates bazel flags from a list of arguments for the given bazel command.
// Returns bazel flags and other arguments as separate lists.
func SeparateBazelFlags(command string, args []string) ([]string, []string, error) {
	flags := bazelFlagSets[command]
	if flags == nil {
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

	otherArgs := make([]string, 0, len(args))
	flagsArgs := make([]string, 0, len(args))

	for len(args) > 0 {
		s := args[0]
		args = args[1:]
		if len(s) == 0 || s[0] != '-' || len(s) == 1 {
			otherArgs = append(otherArgs, s)
			if command == "startup" {
				// special case startup flags which must come before the first command
				otherArgs = append(otherArgs, args...)
				break
			}
			continue
		}

		if s[1] == '-' {
			if len(s) == 2 { // "--" terminates the flags
				otherArgs = append(otherArgs, args...)
				break
			}
			// long arg
			name := s[2:]
			if len(name) == 0 || name[0] == '-' || name[0] == '=' {
				return nil, nil, fmt.Errorf("bad flag syntax: %s", s)
			}
			split := strings.SplitN(name, "=", 2)
			name = split[0]
			flag := flags.Lookup(name)
			if name == "version" || name == "bazel-version" || name == "help" {
				// --version, --bazel-version and --help special cases
				otherArgs = append(otherArgs, s)
			} else if strings.HasPrefix(name, "aspect:") {
				// --aspect:* special case
				otherArgs = append(otherArgs, s)
			} else if flag == nil {
				// --unknown_flag special case
				// This could be a dynamically flag such as --@io_bazel_rules_go//go/config:pure.
				// We can only assume that this flag we don't recognize does not take a value.
				otherArgs = append(otherArgs, s)
			} else if len(split) == 2 {
				// '--flag=arg'
				flagsArgs = append(flagsArgs, s)
			} else if flag.NoOptDefVal != "" {
				// '--flag' (arg was optional)
				flagsArgs = append(flagsArgs, s)
			} else if len(args) > 0 {
				// '--flag arg'
				flagsArgs = append(flagsArgs, s)
				flagsArgs = append(flagsArgs, args[0])
				args = args[1:]
			} else {
				// '--flag' (arg was required)
				return nil, nil, fmt.Errorf("flag needs an argument: %s", s)
			}
		} else {
			// short arg
			if s == "-v" || s == "-h" {
				// -v and -h special cases
				otherArgs = append(otherArgs, s)
			}
			flagsArgs = append(flagsArgs, s)
		}
	}

	return otherArgs, flagsArgs, nil
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
