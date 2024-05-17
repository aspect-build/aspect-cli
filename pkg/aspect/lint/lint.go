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

package lint

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"strconv"
	"strings"

	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/bazel/workspace"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system/bep"
	"github.com/bluekeyes/go-gitdiff/gitdiff"
	"github.com/fatih/color"
	"github.com/manifoldco/promptui"
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
	"github.com/spf13/viper"
	"golang.org/x/sync/errgroup"
)

type Linter struct {
	ioutils.Streams
	bzl bazel.Bazel
}

// Align with rules_lint
const (
	LINT_REPORT_GROUP = "rules_lint_report"
	LINT_PATCH_GROUP  = "rules_lint_patch"
	LINT_RESULT_REGEX = ".*aspect_rules_lint.*"
)

func New(
	streams ioutils.Streams,
	bzl bazel.Bazel,
) *Linter {
	return &Linter{
		Streams: streams,
		bzl:     bzl,
	}
}

func AddFlags(flags *pflag.FlagSet) {
	flags.Bool("fix", false, "Apply patch fixes for lint errors")
	flags.Bool("diff", false, "Output patch fixes for lint errors")
	flags.Bool("report", true, "Output lint reports")
}

// TODO: hoist this to a flags package so it can be used by other commands that require this functionality
func separateFlags(flags *pflag.FlagSet, args []string) ([]string, []string, error) {
	flagsArgs := make([]string, 0, len(args))
	otherArgs := make([]string, 0, len(args))

	for len(args) > 0 {
		s := args[0]
		args = args[1:]
		if len(s) == 0 || s[0] != '-' || len(s) == 1 {
			otherArgs = append(otherArgs, s)
			continue
		}

		name := s[1:]
		if s[1] == '-' {
			if len(s) == 2 { // "--" terminates the flags
				otherArgs = append(otherArgs, args...)
				break
			}
			// long arg with double dash
			name = s[2:]
		}
		if len(name) == 0 || name[0] == '-' || name[0] == '=' {
			return nil, nil, fmt.Errorf("bad flag syntax: %s", s)
		}
		split := strings.SplitN(name, "=", 2)
		name = split[0]
		flag := flags.Lookup(name)
		if flag == nil {
			otherArgs = append(otherArgs, s)
		} else if len(split) == 2 {
			// '-f=arg' or '--flag=arg'
			flagsArgs = append(flagsArgs, s)
		} else if flag.NoOptDefVal != "" {
			// '-f' or '--flag' (arg was optional)
			flagsArgs = append(flagsArgs, s)
		} else if len(args) > 0 {
			// '-f arg' or '--flag arg'
			flagsArgs = append(flagsArgs, s)
			flagsArgs = append(flagsArgs, args[0])
			args = args[1:]
		} else {
			// '-f' or '--flag' (arg was required)
			return nil, nil, fmt.Errorf("flag needs an argument: %s", s)
		}
	}

	return flagsArgs, otherArgs, nil
}

func (runner *Linter) Run(ctx context.Context, cmd *cobra.Command, args []string) error {
	isInteractiveMode, err := cmd.Root().PersistentFlags().GetBool(flags.AspectInteractiveFlagName)
	linters := viper.GetStringSlice("lint.aspects")

	if len(linters) == 0 {
		fmt.Fprintf(runner.Streams.Stdout, `No aspects enabled for linting.
		
Add a section like the following to your .aspect/cli/config.yaml:

lint:
  aspects:
    - //tools:lint.bzl%%eslint
`)
		return nil
	}

	// Get values of lint command specific flags
	applyFix, _ := cmd.Flags().GetBool("fix")
	printDiff, _ := cmd.Flags().GetBool("diff")
	printReport, _ := cmd.Flags().GetBool("report")

	// Separate out the lint command specific flags from the list of args to
	// pass to `bazel build`
	lintFlagSet := pflag.NewFlagSet("lint", pflag.ContinueOnError)
	AddFlags(lintFlagSet)
	_, bazelArgs, err := separateFlags(lintFlagSet, args)
	if err != nil {
		return fmt.Errorf("failed to parse lint flags: %w", err)
	}

	// Construct the `bazel build` command
	bazelCmd := []string{"build"}
	bazelCmd = append(bazelCmd, bazelArgs...)
	bazelCmd = append(bazelCmd, fmt.Sprintf("--aspects=%s", strings.Join(linters, ",")))

	// optimization: don't request report files in a mode where we don't print them
	outputGroups := []string{}
	if applyFix || printDiff || isInteractiveMode {
		bazelCmd = append(bazelCmd, "--@aspect_rules_lint//lint:fix")
		outputGroups = append(outputGroups, LINT_PATCH_GROUP)
	}
	if printReport {
		outputGroups = append(outputGroups, LINT_REPORT_GROUP)
	}

	bazelCmd = append(bazelCmd, fmt.Sprintf("--output_groups=%s", strings.Join(outputGroups, ",")))

	// Don't trigger Validation Actions along with lint reports.
	// > The validations output group "is special in that its outputs are always requested, regardless of the value of the
	// > --output_groups flag, and regardless of how the target is depended upon"
	// https://bazel.build/extending/rules#validations_output_group
	bazelCmd = append(bazelCmd, "--run_validations=false")

	// TODO: in Bazel 7 this was renamed without the --experimental_ prefix
	bazelCmd = append(bazelCmd, fmt.Sprintf("--experimental_remote_download_regex='%s'", LINT_RESULT_REGEX))

	handleResultsErrgroup, handleResultsCtx := errgroup.WithContext(context.Background())

	// Currently Bazel only supports a single --bes_backend so adding ours after
	// any user supplied value will result in our bes_backend taking precedence.
	// There is a very old & stale issue to add support for multiple BES
	// backends https://github.com/bazelbuild/bazel/issues/10908. In the future,
	// we could build this support into the Aspect CLI and post on that issue
	// that using the Aspect CLI resolves it.
	var lintBEPHandler *LintBEPHandler
	if bep.HasBESBackend(ctx) {
		besBackend := bep.BESBackendFromContext(ctx)
		besBackendFlag := fmt.Sprintf("--bes_backend=%s", besBackend.Addr())
		bazelCmd = flags.AddFlagToCommand(bazelCmd, besBackendFlag)

		workingDirectory, err := os.Getwd()
		if err != nil {
			return fmt.Errorf("failed to get current working directory: %w", err)
		}
		finder := workspace.DefaultFinder
		workspaceRoot, err := finder.Find(workingDirectory)
		if err != nil {
			return fmt.Errorf("failed to find workspace root: %w", err)
		}

		lintBEPHandler = newLintBEPHandler(handleResultsCtx, workspaceRoot, handleResultsErrgroup)
		besBackend.RegisterSubscriber(lintBEPHandler.BEPEventCallback)
	}

	err = runner.bzl.RunCommand(runner.Streams, nil, bazelCmd...)

	// Wait for completion and return the first error (if any)
	wgErr := handleResultsErrgroup.Wait()
	if wgErr != nil && err == nil {
		return wgErr
	}

	// Check for subscriber errors
	subscriberErrors := bep.BESErrors(ctx)
	if len(subscriberErrors) > 0 {
		for _, err := range subscriberErrors {
			fmt.Fprintf(runner.Streams.Stderr, "Error: failed to run lint command: %v\n", err)
		}
		if err == nil {
			return fmt.Errorf("%v BES subscriber error(s)", len(subscriberErrors))
		}
	}

	// Bazel is done running, so stdout is now safe for us to print the results
	applyAll := false
	applyNone := false
	exitCode := 0
	for label, result := range lintBEPHandler.resultsByLabel {
		l := label
		if result.exitCodeFile != nil {
			exitCodeStr, err := lintBEPHandler.readBEPFile(result.exitCodeFile)
			if err != nil {
				return err
			}
			targetExitCode, err := strconv.Atoi(strings.TrimSpace(exitCodeStr))
			if err != nil {
				return fmt.Errorf("failed parse read exit code as integer: %v", err)
			}
			if targetExitCode > 0 {
				exitCode = 1
			}
		}
		f := result.reportFile
		content, err := lintBEPHandler.readBEPFile(f)
		if err != nil {
			return err
		}
		if applyFix || printDiff {
			runner.patchLintResult(label, content, applyFix, printDiff)
		}
		if printReport {
			runner.outputLintResultText(l, content)

			if isInteractiveMode && result.patchFile != nil && !applyNone {
				var choice string
				if applyAll {
					choice = "y"
				} else {
					applyFixPrompt := promptui.Prompt{
						Label:   "Apply fixes? [y]es / [n]o / [A]ll / [N]one",
						Default: "y",
					}
					choice, err = applyFixPrompt.Run()
					if err != nil {
						return fmt.Errorf("prompt failed: %v", err)
					}
				}
				if strings.HasPrefix(choice, "A") {
					applyAll = true
				}
				if strings.HasPrefix(choice, "N") {
					applyNone = true
				}
				if applyAll || strings.HasPrefix(choice, "y") {
					patchResult, err := lintBEPHandler.readBEPFile(result.patchFile)
					if err != nil {
						return err
					}
					runner.patchLintResult(label, patchResult, true, false)
				}
			}
		}
	}

	return &aspecterrors.ExitError{ExitCode: exitCode}
}

func (runner *Linter) patchLintResult(label string, lintPatch string, applyDiff, printDiff bool) error {
	if printDiff {
		color.New(color.FgYellow).Fprintf(runner.Streams.Stdout, "From %s:\n", label)
		fmt.Fprintf(runner.Streams.Stdout, "%s\n", lintPatch)
	}

	if applyDiff {
		files, _, err := gitdiff.Parse(strings.NewReader(lintPatch))
		if err != nil {
			return err
		}

		for _, file := range files {
			// TODO: file.IsNew|IsDeleted|IsRename|IsCopy

			oldSrc, openErr := os.OpenFile(file.OldName[2:], os.O_RDONLY, 0)
			if openErr != nil {
				return openErr
			}
			defer oldSrc.Close()

			var output bytes.Buffer
			applyErr := gitdiff.Apply(&output, oldSrc, file)
			if applyErr != nil {
				return applyErr
			}

			writeErr := os.WriteFile(file.NewName[2:], output.Bytes(), file.NewMode.Perm())
			if writeErr != nil {
				return writeErr
			}
			color.New(color.Faint).Fprintf(runner.Streams.Stdout, "Patched %s\n", file.NewName[2:])
		}
	}

	return nil
}

func (runner *Linter) outputLintResultText(label string, lineResult string) error {
	lineResult = strings.TrimSpace(lineResult)
	if len(lineResult) > 0 {
		color.New(color.FgYellow).Fprintf(runner.Streams.Stdout, "From %s:\n", label)
		fmt.Fprintf(runner.Streams.Stdout, "%s\n", lineResult)
		fmt.Fprintln(runner.Streams.Stdout, "")
	}
	return nil
}
