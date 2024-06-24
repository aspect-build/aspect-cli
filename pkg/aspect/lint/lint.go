/*
 * Copyright 2023 Aspect Build Systems, Inc.
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
	"math"
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
	"github.com/charmbracelet/huh"
	"github.com/fatih/color"
	godiff "github.com/sourcegraph/go-diff/diff"
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
	"github.com/spf13/viper"
	"golang.org/x/sync/errgroup"
)

type LintResult struct {
	Label    string
	Mnemonic string
	ExitCode int
	Report   string
	Patch    []byte
}

type LintResultsHandler interface {
	Results(cmd *cobra.Command, results []*LintResult) error
	AddFlags(flags *pflag.FlagSet)
}

type Linter struct {
	ioutils.Streams
	bzl             bazel.Bazel
	resultsHandlers []LintResultsHandler
}

// Align with rules_lint
const (
	LINT_REPORT_GROUP  = "rules_lint_report"
	LINT_PATCH_GROUP   = "rules_lint_patch"
	LINT_RESULT_REGEX  = ".*AspectRulesLint.*"
	HISTOGRAM_CHARS    = 20
	MAX_FILENAME_WIDTH = 80
)

func New(
	streams ioutils.Streams,
	bzl bazel.Bazel,
	resultsHandlers []LintResultsHandler,
) *Linter {
	return &Linter{
		Streams:         streams,
		bzl:             bzl,
		resultsHandlers: resultsHandlers,
	}
}

func AddFlags(flags *pflag.FlagSet) {
	flags.Bool("fix", false, "Apply all patch fixes for lint violations")
	flags.Bool("diff", false, "Show unified diff instead of diff stats for fixes")
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
	applyAll, _ := cmd.Flags().GetBool("fix")
	showDiff, _ := cmd.Flags().GetBool("diff")
	printReport, _ := cmd.Flags().GetBool("report")

	// Separate out the lint command specific flags from the list of args to
	// pass to `bazel build`
	lintFlagSet := pflag.NewFlagSet("lint", pflag.ContinueOnError)
	AddFlags(lintFlagSet)
	for _, h := range runner.resultsHandlers {
		h.AddFlags(lintFlagSet)
	}
	_, bazelArgs, err := separateFlags(lintFlagSet, args)
	if err != nil {
		return fmt.Errorf("failed to parse lint flags: %w", err)
	}

	// Construct the `bazel build` command
	bazelCmd := []string{"build"}
	bazelCmd = append(bazelCmd, bazelArgs...)
	bazelCmd = append(bazelCmd, fmt.Sprintf("--aspects=%s", strings.Join(linters, ",")))

	// Don't request report and patch files in a mode where we don't need them
	outputGroups := []string{}
	if printReport || applyAll || isInteractiveMode {
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

	var downloadFlag = "--experimental_remote_download_regex"

	// --experimental_remote_download_regex was deprecated in Bazel 7 in favor of
	// --remote_download_regex. Use the latter if it is a valid flag so we don't see the warning:
	// WARNING: Option 'experimental_remote_download_regex' is deprecated: Use --remote_download_regex instead
	useShortDownloadFlag, err := runner.bzl.IsBazelFlag("build", "remote_download_regex")
	if err != nil {
		return fmt.Errorf("failed to check for bazel flag --remote_download_regex: %w", err)
	}
	if useShortDownloadFlag {
		downloadFlag = "--remote_download_regex"
	}

	bazelCmd = append(bazelCmd, fmt.Sprintf("%s='%s'", downloadFlag, LINT_RESULT_REGEX))

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
		besBackend.RegisterSubscriber(lintBEPHandler.bepEventCallback)
	}

	err = runner.bzl.RunCommand(runner.Streams, nil, bazelCmd...)
	if err != nil {
		return err
	}

	// Wait for completion and return the first error (if any)
	wgErr := handleResultsErrgroup.Wait()
	if wgErr != nil {
		return wgErr
	}

	// Check for subscriber errors
	subscriberErrors := bep.BESErrors(ctx)
	if len(subscriberErrors) > 0 {
		for _, err := range subscriberErrors {
			fmt.Fprintf(runner.Streams.Stderr, "Error: failed to run lint command: %v\n", err)
		}
		return fmt.Errorf("%v BES subscriber error(s)", len(subscriberErrors))
	}

	// Convert raw results to list of LintResult structs
	results := make([]*LintResult, 0, len(lintBEPHandler.resultsByLabel))
	for _, r := range lintBEPHandler.resultsByLabel {
		result := &LintResult{
			Mnemonic: r.mnemonic,
			Label:    r.label,
		}
		results = append(results, result)

		// parse exit code file
		if r.exitCodeFile != nil {
			exitCodeBytes, err := lintBEPHandler.readBEPFile(r.exitCodeFile)
			if err != nil {
				return err
			}
			exitCode, err := strconv.Atoi(strings.TrimSpace(string(exitCodeBytes)))
			if err != nil {
				return fmt.Errorf("failed parse read exit code as integer: %v", err)
			}
			result.ExitCode = exitCode
		}

		// read the report file
		if r.reportFile != nil {
			reportBytes, err := lintBEPHandler.readBEPFile(r.reportFile)
			if err != nil {
				return err
			}
			result.Report = strings.TrimSpace(string(reportBytes))
		}

		// read the patch file
		if r.patchFile != nil {
			patch, err := lintBEPHandler.readBEPFile(r.patchFile)
			if err != nil {
				return err
			}
			if patch != nil && len(patch) > 0 {
				result.Patch = patch
			}
		}
	}

	// Send the result to any lint handlers. Call the handlers even if results list
	// is empty since no results is a success.
	for _, h := range runner.resultsHandlers {
		if err := h.Results(cmd, results); err != nil {
			return fmt.Errorf("lint results handler failed: %w", err)
		}
	}

	// Bazel is done running, so stdout is now safe for us to print the results
	applyNone := false
	exitCode := 0
	for _, r := range results {
		if r.ExitCode > 0 {
			exitCode = 1
		}

		printHeader := true
		if len(r.Report) > 0 {
			if printHeader {
				runner.printLintResultsHeader(r.Label)
				printHeader = false
			}
			runner.printLintReport(r.Report)
		}

		if r.Patch != nil {
			if printHeader {
				runner.printLintResultsHeader(r.Label)
				printHeader = false
			}
			color.New(color.FgYellow).Fprintf(runner.Streams.Stdout, "Some problems have automated fixes available:\n\n")
			if showDiff {
				runner.printLintPatchDiff(r.Patch)
			} else {
				err = runner.printLintPatchDiffStat(r.Patch)
				if err != nil {
					return fmt.Errorf("failed to parse patch file for %s: %v", r.Label, err)
				}
			}
			apply := applyAll
			if isInteractiveMode && !applyNone && !apply {
				for {
					var choice string
					options := []huh.Option[string]{
						huh.NewOption("Yes", "yes"),
						huh.NewOption("No", "no"),
						huh.NewOption("All", "all"),
						huh.NewOption("None", "none"),
					}
					if !showDiff {
						options = append(options, huh.NewOption("Show Diff", "diff"))
					}
					applyFixPrompt := huh.NewSelect[string]().
						Title("Apply fixes?").
						Options(options...).
						Value(&choice)
					form := huh.NewForm(huh.NewGroup(applyFixPrompt))
					err := form.Run()
					if err != nil {
						return fmt.Errorf("prompt failed: %v", err)
					}
					switch choice {
					case "yes":
						apply = true
					case "all":
						apply = true
						applyAll = true
					case "none":
						applyNone = true
					case "diff":
						runner.printLintPatchDiff(r.Patch)
						continue
					}
					break
				}
			}
			if apply {
				err = runner.applyLintPatch(r.Patch)
				if err != nil {
					return fmt.Errorf("failed to apply patch file for %s: %v", r.Label, err)
				}
			}
		}
	}

	return &aspecterrors.ExitError{ExitCode: exitCode}
}

func (runner *Linter) printLintResultsHeader(label string) {
	color.New(color.Bold).Fprintf(runner.Streams.Stdout, "Lint results for %s:\n\n", label)
}

func (runner *Linter) printLintReport(report string) {
	fmt.Fprintf(runner.Streams.Stdout, "%s\n", report)
	fmt.Fprintln(runner.Streams.Stdout, "")
}

type diffSummary struct {
	name    string
	added   int
	deleted int
	changed int
	total   int
}

// Prints an output similar to `git diff | diffstat -m -C`.
// See https://invisible-island.net/diffstat/.
//
// For example,
//
// $ git diff | diffstat -m -C
//
//	e2e/pnpm_lockfiles/README.md                     |    2
//	e2e/pnpm_lockfiles/base/package.json             |    5 !!
//	e2e/pnpm_lockfiles/lockfile-test.bzl             |    3 !
//	e2e/pnpm_lockfiles/setup.sh                      |    4 !
//	e2e/pnpm_lockfiles/update-snapshots.sh           |    6 ++
//	e2e/pnpm_lockfiles/v54/pnpm-lock.yaml            |   44 +++++++++++++++++!!
//	e2e/pnpm_lockfiles/v54/snapshots/defs.bzl        |   78 ++++++++++++++++++++!!!!!!!!!!!!!!
//	e2e/pnpm_lockfiles/v60/pnpm-lock.yaml            |   50 +++++++++++++++++++-!!
//	e2e/pnpm_lockfiles/v60/snapshots/defs.bzl        |   81 ++++++++++++++++++++!!!!!!!!!!!!!!!
//	e2e/pnpm_lockfiles/v61/pnpm-lock.yaml            |   50 +++++++++++++++++++-!!
//	e2e/pnpm_lockfiles/v61/snapshots/defs.bzl        |   81 ++++++++++++++++++++!!!!!!!!!!!!!!!
//	e2e/pnpm_lockfiles/v90/pnpm-lock.yaml            |   51 +++++++++++++++++--!!
//	e2e/pnpm_lockfiles/v90/snapshots/defs.bzl        |   81 ++++++++++++++++++++++!!!!!!!!!!!!!
//	npm/private/test/parse_pnpm_lock_tests.bzl       |  138 ++++++++++++++++++++++++++++++++++++++++-------------------
//	npm/private/test/snapshots/wksp/repositories.bzl |    4 !
//	npm/private/test/transitive_closure_tests.bzl    |    7 !!!
//	npm/private/test/utils_tests.bzl                 |    6 !!
//	npm/private/transitive_closure.bzl               |   89 --------------------------------!!!!!!!
//	npm/private/utils.bzl                            |  301 +++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++---!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
//	19 files changed, 664 insertions(+), 134 deletions(-), 283 modifications(!)
func (runner *Linter) printLintPatchDiffStat(patch []byte) error {
	diffs, err := godiff.ParseMultiFileDiff(patch)
	if err != nil {
		return err
	}
	sumAdded := 0
	sumDeleted := 0
	sumChanged := 0
	maxLines := 1
	nameColumn := 1
	summaries := make([]*diffSummary, 0, len(diffs))

	for _, diff := range diffs {
		stat := diff.Stat()

		summary := new(diffSummary)
		// strip the a/ and b/ from the names
		summary.name = diff.OrigName[2:]
		newName := diff.NewName[2:]
		// we're not expecting linters to rename files but we can't be sure we won't
		// get a diff with a rename so we handle that case properly
		if summary.name != newName {
			summary.name = summary.name + " => " + newName
		}
		newName = diff.NewName[2:]
		summary.added = int(stat.Added)
		summary.deleted = int(stat.Deleted)
		summary.changed = int(stat.Changed)
		summary.total = summary.added + summary.changed + summary.deleted
		summaries = append(summaries, summary)

		// add up totals and find maximums
		sumAdded += summary.added
		sumDeleted += summary.deleted
		sumChanged += summary.changed
		if summary.total > maxLines {
			maxLines = summary.total
		}
		if len(summary.name) > nameColumn {
			nameColumn = min(len(summary.name), MAX_FILENAME_WIDTH)
		}
	}

	linesColumn := len(fmt.Sprint(maxLines))
	histChars := min(HISTOGRAM_CHARS, maxLines)
	for _, summary := range summaries {
		histAdded := int(math.Floor(float64(summary.added) / float64(maxLines) * float64(histChars)))
		if summary.added > 0 && histAdded == 0 {
			histAdded = 1
		}
		histDeleted := int(math.Floor(float64(summary.deleted) / float64(maxLines) * float64(histChars)))
		if summary.deleted > 0 && histDeleted == 0 {
			histDeleted = 1
		}
		histChanged := int(math.Floor(float64(summary.changed) / float64(maxLines) * float64(histChars)))
		if summary.changed > 0 && histChanged == 0 {
			histChanged = 1
		}
		name := summary.name
		if len(name) > nameColumn {
			// truncate long filenames
			name = "..." + name[len(name)-nameColumn+3:]
		}
		fmt.Fprintf(runner.Streams.Stdout, "  %-*s | ", nameColumn, name)
		fmt.Fprintf(runner.Streams.Stdout, "%*d ", linesColumn, summary.total)
		color.New(color.FgGreen).Fprint(runner.Streams.Stdout, strings.Repeat("+", histAdded))
		color.New(color.FgRed).Fprint(runner.Streams.Stdout, strings.Repeat("-", histDeleted))
		color.New(color.FgCyan).Fprint(runner.Streams.Stdout, strings.Repeat("!", histChanged))
		fmt.Fprintln(runner.Streams.Stdout, "")
	}

	// 1 file, 1 insertion(+), 5 deletions(-), 1 modification(!)
	fmt.Fprintf(runner.Streams.Stdout, "  %d file%s", len(summaries), strings.Repeat("s", min(1, len(summaries)-1)))
	if sumAdded > 0 {
		fmt.Fprintf(runner.Streams.Stdout, ", %d insertion%s(+)", sumAdded, strings.Repeat("s", min(1, sumAdded-1)))
	}
	if sumDeleted > 0 {
		fmt.Fprintf(runner.Streams.Stdout, ", %d deletion%s(-)", sumDeleted, strings.Repeat("s", min(1, sumDeleted-1)))
	}
	if sumChanged > 0 {
		fmt.Fprintf(runner.Streams.Stdout, ", %d modification%s(!)", sumChanged, strings.Repeat("s", min(1, sumChanged-1)))
	}
	fmt.Fprintln(runner.Streams.Stdout, "")
	fmt.Fprintln(runner.Streams.Stdout, "")
	return nil
}

func (runner *Linter) printLintPatchDiff(patch []byte) {
	fmt.Fprint(runner.Streams.Stdout, string(patch))
	fmt.Fprintln(runner.Streams.Stdout, "")
}

func (runner *Linter) applyLintPatch(patch []byte) error {
	files, _, err := gitdiff.Parse(bytes.NewBuffer(patch))
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

	if len(files) > 0 {
		fmt.Fprintln(runner.Streams.Stdout, "")
	}

	return nil
}
