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
	"os"
	"path/filepath"
	"strings"

	"aspect.build/cli/bazel/buildeventstream"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system/bep"
	"github.com/bluekeyes/go-gitdiff/gitdiff"
	"github.com/fatih/color"
	"github.com/reviewdog/errorformat"
	"github.com/reviewdog/errorformat/fmts"
	"github.com/reviewdog/errorformat/writer"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"
)

type Linter struct {
	ioutils.Streams
	bzl bazel.Bazel
}

type LintBEPHandler struct {
	ioutils.Streams
	report  bool
	fix     bool
	diff    bool
	output  string
	reports map[string]*buildeventstream.NamedSetOfFiles
}

// Align with rules_lint
const (
	LINT_REPORT_GROUP = "rules_lint_report"
	LINT_PATCH_GROUP  = "rules_lint_patch"
	LINT_RESULT_REGEX = ".*aspect_rules_lint.*"
)

func newLintBEPHandler(streams ioutils.Streams, printReport, applyFix, printDiff bool, output string) *LintBEPHandler {
	return &LintBEPHandler{
		Streams: streams,
		report:  printReport,
		fix:     applyFix,
		diff:    printDiff,
		output:  output,
		reports: make(map[string]*buildeventstream.NamedSetOfFiles),
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

	applyFix, _ := cmd.Flags().GetBool("fix")
	printDiff, _ := cmd.Flags().GetBool("diff")
	printReport, _ := cmd.Flags().GetBool("report")
	output, _ := cmd.Flags().GetString("output")

	bazelCmd := []string{"build"}
	bazelCmd = append(bazelCmd, fmt.Sprintf("--aspects=%s", strings.Join(linters, ",")))
	bazelCmd = append(bazelCmd, cmd.Flags().Args()...)

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

		lintBEPHandler := newLintBEPHandler(runner.Streams, printReport, applyFix, printDiff, output)
		besBackend.RegisterSubscriber(lintBEPHandler.BEPEventCallback)
	}

	err := runner.bzl.RunCommand(runner.Streams, nil, bazelCmd...)

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
					for _, f := range fileSet.GetFiles() {

						var err error
						if outputGroup.Name == LINT_PATCH_GROUP {
							err = runner.patchLintResult(label, f, runner.fix, runner.diff)
						} else if outputGroup.Name == LINT_REPORT_GROUP && runner.report {
							switch runner.output {
							case "text":
								err = runner.outputLintResultText(label, f)
							case "sarif":
								// FIXME: print one serif document, not one per report file
								err = runner.outputLintResultSarif(label, f)
							default:
								err = fmt.Errorf("Unsupported output kind %s", runner.output)
							}
						}

						if err != nil {
							return err
						}
					}

					runner.reports[fileSetId.Id] = nil
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

func (runner *LintBEPHandler) readBEPFile(f *buildeventstream.File) (string, error) {
	// TODO: use f.GetContents()?

	if strings.HasPrefix(f.GetUri(), "file://") {
		localFilePath := strings.TrimPrefix(f.GetUri(), "file://")
		lintResultBuf, err := os.ReadFile(localFilePath)
		if err != nil {
			return "", err
		}
		return string(lintResultBuf), nil
	}

	// TODO: support bytestream://

	return "", fmt.Errorf("failed to extract lint result file")
}
