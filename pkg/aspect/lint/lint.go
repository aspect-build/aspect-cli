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
	"log"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"strings"
	"time"

	"aspect.build/cli/bazel/buildeventstream"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/bazel/workspace"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system/bep"
	"github.com/bluekeyes/go-gitdiff/gitdiff"
	"github.com/fatih/color"
	"github.com/reviewdog/errorformat"
	"github.com/reviewdog/errorformat/fmts"
	"github.com/reviewdog/errorformat/writer"
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
	"github.com/spf13/viper"
	"golang.org/x/sync/errgroup"
)

type Linter struct {
	ioutils.Streams
	bzl bazel.Bazel
}

type LintBEPHandler struct {
	ioutils.Streams
	ctx                   context.Context
	report                bool
	fix                   bool
	diff                  bool
	output                string
	reports               map[string]*buildeventstream.NamedSetOfFiles
	workspaceRoot         string
	handleResultsErrgroup *errgroup.Group
}

// Align with rules_lint
const (
	LINT_REPORT_GROUP = "rules_lint_report"
	LINT_PATCH_GROUP  = "rules_lint_patch"
	LINT_RESULT_REGEX = ".*aspect_rules_lint.*"
)

func newLintBEPHandler(ctx context.Context, streams ioutils.Streams, printReport, applyFix, printDiff bool, output string, workspaceRoot string, handleResultsErrgroup *errgroup.Group) *LintBEPHandler {
	return &LintBEPHandler{
		Streams:               streams,
		ctx:                   ctx,
		report:                printReport,
		fix:                   applyFix,
		diff:                  printDiff,
		output:                output,
		reports:               make(map[string]*buildeventstream.NamedSetOfFiles),
		workspaceRoot:         workspaceRoot,
		handleResultsErrgroup: handleResultsErrgroup,
	}
}

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
	flags.String("output", "text", "Format for output of lint reports, either 'text' or 'sarif'")
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
	output, _ := cmd.Flags().GetString("output")

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

	outputGroups := []string{}
	if applyFix || printDiff {
		outputGroups = append(outputGroups, LINT_PATCH_GROUP)
	}
	if printReport {
		outputGroups = append(outputGroups, LINT_REPORT_GROUP)
	}
	bazelCmd = append(bazelCmd, fmt.Sprintf("--output_groups=%s", strings.Join(outputGroups, ",")))
	// TODO: in Bazel 7 this was renamed without the --experimental_ prefix
	bazelCmd = append(bazelCmd, fmt.Sprintf("--experimental_remote_download_regex='%s'", LINT_RESULT_REGEX))

	handleResultsErrgroup, handleResultsCtx := errgroup.WithContext(context.Background())

	// Currently Bazel only supports a single --bes_backend so adding ours after
	// any user supplied value will result in our bes_backend taking precedence.
	// There is a very old & stale issue to add support for multiple BES
	// backends https://github.com/bazelbuild/bazel/issues/10908. In the future,
	// we could build this support into the Aspect CLI and post on that issue
	// that using the Aspect CLI resolves it.
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

		lintBEPHandler := newLintBEPHandler(handleResultsCtx, runner.Streams, printReport, applyFix, printDiff, output, workspaceRoot, handleResultsErrgroup)
		besBackend.RegisterSubscriber(lintBEPHandler.BEPEventCallback)
	}

	err = runner.bzl.RunCommand(runner.Streams, nil, bazelCmd...)

	// Wait for completion and return the first error (if any)
	wgErr := handleResultsErrgroup.Wait()
	if wgErr != nil && err == nil {
		err = wgErr
	}

	// Check for subscriber errors
	subscriberErrors := bep.BESErrors(ctx)
	if len(subscriberErrors) > 0 {
		for _, err := range subscriberErrors {
			fmt.Fprintf(runner.Streams.Stderr, "Error: failed to run lint command: %v\n", err)
		}
		if err == nil {
			err = fmt.Errorf("%v BES subscriber error(s)", len(subscriberErrors))
		}
	}

	return err
}

func (runner *LintBEPHandler) BEPEventCallback(event *buildeventstream.BuildEvent) error {
	switch event.Payload.(type) {

	case *buildeventstream.BuildEvent_NamedSetOfFiles:
		// Assert no collisions
		namedSetId := event.Id.GetNamedSet().GetId()
		if runner.reports[namedSetId] != nil {
			return fmt.Errorf("duplicate file set id: %s", namedSetId)
		}

		// Record report file sets
		// TODO: are we collecting too much? Don't we need to filter on the "report" output group?
		runner.reports[namedSetId] = event.GetNamedSetOfFiles()

	case *buildeventstream.BuildEvent_Completed:
		label := event.Id.GetTargetCompleted().GetLabel()

		for _, outputGroup := range event.GetCompleted().OutputGroup {
			for _, fileSetId := range outputGroup.FileSets {
				if fileSet := runner.reports[fileSetId.Id]; fileSet != nil {
					runner.reports[fileSetId.Id] = nil

					for _, file := range fileSet.GetFiles() {
						l := label
						f := file
						if outputGroup.Name == LINT_PATCH_GROUP {
							runner.handleResultsErrgroup.Go(func() error {
								return runner.patchLintResult(l, f, runner.fix, runner.diff)
							})
						} else if outputGroup.Name == LINT_REPORT_GROUP && runner.report {
							switch runner.output {
							case "text":
								runner.handleResultsErrgroup.Go(func() error {
									return runner.outputLintResultText(l, f)
								})
							case "sarif":
								runner.handleResultsErrgroup.Go(func() error {
									return runner.outputLintResultSarif(l, f)
								})
							default:
								return fmt.Errorf("unsupported output kind %s", runner.output)
							}
						}
					}
				}
			}
		}
	}

	return nil
}

func (runner *LintBEPHandler) patchLintResult(label string, f *buildeventstream.File, applyDiff, printDiff bool) error {
	lintPatch, err := runner.readBEPFile(f)
	if err != nil {
		return err
	}

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
		}
	}

	return nil
}

func (runner *LintBEPHandler) outputLintResultText(label string, f *buildeventstream.File) error {
	lineResult, err := runner.readBEPFile(f)
	if err != nil {
		return err
	}

	lineResult = strings.TrimSpace(lineResult)
	if len(lineResult) > 0 {
		color.New(color.FgYellow).Fprintf(runner.Streams.Stdout, "From %s:\n", label)
		fmt.Fprintf(runner.Streams.Stdout, "%s\n", lineResult)
	}

	return nil
}

func (runner *LintBEPHandler) outputLintResultSarif(label string, f *buildeventstream.File) error {
	lineResult, err := runner.readBEPFile(f)
	if err != nil {
		return err
	}

	// Parse the filename convention that rules_lint has for report files.
	// path/to/linter.target_name.aspect_rules_lint.report -> linter
	linter := strings.SplitN(filepath.Base(f.Name), ".", 2)[0]
	var fm []string

	// Switch is on the MNEMONIC declared in rules_lint
	switch linter {
	// TODO: ESLint
	case "flake8":
		fm = fmts.DefinedFmts()["flake8"].Errorformat
	case "PMD":
		// TODO: upstream to https://github.com/reviewdog/errorformat/issues/62
		fm = []string{`%f:%l:\\t%m`}
	case "ruff":
		fm = []string{
			`%f:%l:%c: %t%n %m`,
			`%-GFound %n error%.%#`,
			`%-G[*] %n fixable%.%#`,
		}
	case "buf":
		fm = []string{
			`--buf-plugin_out: %f:%l:%c:%m`,
		}
	case "Vale":
		fm = []string{`%f:%l:%c:%m`}
	default:
		fmt.Fprintf(runner.Streams.Stderr, "No format string for linter %s\n", linter)
	}

	if fm == nil {
		return nil
	}
	efm, err := errorformat.NewErrorformat(fm)
	if err != nil {
		return err
	}

	var ewriter writer.Writer
	var sarifOpt writer.SarifOption
	sarifOpt.ToolName = linter
	ewriter, err = writer.NewSarif(runner.Streams.Stdout, sarifOpt)
	if err != nil {
		return err
	}
	if ewriter, ok := ewriter.(writer.BufWriter); ok {
		defer func() {
			if err := ewriter.Flush(); err != nil {
				log.Println(err)
			}
		}()
	}

	s := efm.NewScanner(strings.NewReader(lineResult))
	for s.Scan() {
		if err := ewriter.Write(s.Entry()); err != nil {
			return err
		}
	}

	return nil
}

func (runner *LintBEPHandler) readBEPFile(file *buildeventstream.File) (string, error) {
	resultsFile := ""

	switch f := file.File.(type) {
	case *buildeventstream.File_Uri:
		uri, err := url.Parse(f.Uri)
		if err != nil {
			return "", fmt.Errorf("unable to parse URI %s: %v", f.Uri, err)
		}
		if uri.Scheme == "file" {
			resultsFile = filepath.Clean(uri.Path)
		} else if uri.Scheme == "bytestream" {
			if strings.HasSuffix(uri.Path, "/0") {
				// No reason to read an empty results file from disk
				return "", nil
			}
			// Because we set --experimental_remote_download_regex, we can depend on the results file being
			// in the output tree even when using a remote cache with build without the bytes.
			resultsFile = path.Join(runner.workspaceRoot, path.Join(file.PathPrefix...), file.Name)
		} else {
			return "", fmt.Errorf("unsupported BES file uri %v", f.Uri)
		}
	default:
		return "", fmt.Errorf("unsupported BES file type")
	}

	start := time.Now()
	for {
		// TODO: also check that the bazel remote cache downloader is not still writing
		// to the results file
		_, err := os.Stat(resultsFile)
		if err != nil {
			// if more than 60s has passed then give up
			// TODO: make this timeout configurable
			if time.Since(start) > 60*time.Second {
				return "", fmt.Errorf("failed to read lint results file %s: %v", resultsFile, err)
			}
		} else {
			buf, err := os.ReadFile(resultsFile)
			if err != nil {
				return "", fmt.Errorf("failed to read lint results file %s: %v", resultsFile, err)
			}
			return string(buf), nil
		}
		// we're in a go routine so yield for 100ms and try again
		// TODO: watch the file system for the file creation instead of polling
		t := time.NewTimer(time.Millisecond * 100)
		select {
		case <-runner.ctx.Done():
			t.Stop()
			return "", fmt.Errorf("failed to read lint results file %s: interrupted", resultsFile)
		case <-t.C:
		}
	}

}
