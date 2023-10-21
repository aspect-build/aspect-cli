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
	"context"
	"fmt"
	"os"
	"strings"

	"aspect.build/cli/bazel/buildeventstream"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system/bep"
	"github.com/fatih/color"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"
)

type Linter struct {
	ioutils.Streams
	bzl           bazel.Bazel
	bazelBin      string
	executionRoot string
	reports       map[string]*buildeventstream.NamedSetOfFiles
}

func New(
	streams ioutils.Streams,
	bzl bazel.Bazel,
) *Linter {
	return &Linter{
		Streams: streams,
		bzl:     bzl,
		reports: make(map[string]*buildeventstream.NamedSetOfFiles),
	}
}

func (runner *Linter) Run(ctx context.Context, _ *cobra.Command, args []string) error {
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

	bazelCmd := []string{"build"}
	bazelCmd = append(bazelCmd, fmt.Sprintf("--aspects=%s", strings.Join(linters, ",")), "--output_groups=rules_lint_report")
	bazelCmd = append(bazelCmd, args...)

	// Currently Bazel only supports a single --bes_backend so adding ours after
	// any user supplied value will result in our bes_backend taking precedence.
	// There is a very old & stale issue to add support for multiple BES
	// backends https://github.com/bazelbuild/bazel/issues/10908. In the future,
	// we could build this support into the Aspect CLI and post on that issue
	// that using the Aspect CLI resolves it.
	if bep.HasBESBackend(ctx) {
		besBackend := bep.BESBackendFromContext(ctx)
		besBackend.RegisterSubscriber(runner.BEPEventCallback)
		besBackendFlag := fmt.Sprintf("--bes_backend=%s", besBackend.Addr())
		bazelCmd = flags.AddFlagToCommand(bazelCmd, besBackendFlag)
	}

	exitCode, bazelErr := runner.bzl.RunCommand(runner.Streams, nil, bazelCmd...)

	// Process the subscribers errors before the Bazel one.
	subscriberErrors := bep.BESErrors(ctx)
	if len(subscriberErrors) > 0 {
		for _, err := range subscriberErrors {
			fmt.Fprintf(runner.Streams.Stderr, "Error: failed to run lint command: %v\n", err)
		}
		exitCode = 1
	}

	if exitCode != 0 {
		err := &aspecterrors.ExitError{ExitCode: exitCode}
		if bazelErr != nil {
			err.Err = bazelErr
		}
		return err
	}

	return nil
}

func (runner *Linter) BEPEventCallback(event *buildeventstream.BuildEvent) error {
	switch event.Payload.(type) {

	case *buildeventstream.BuildEvent_WorkspaceInfo:
		// Record workspace information
		runner.executionRoot = event.GetWorkspaceInfo().GetLocalExecRoot()

	case *buildeventstream.BuildEvent_Configuration:
		// Record configuration information
		runner.bazelBin = event.GetConfiguration().GetMakeVariable()["BINDIR"]

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
						err := runner.outputLintResult(label, f)
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

func (runner *Linter) outputLintResult(label string, f *buildeventstream.File) error {
	lineResult, err := runner.readLintResultFile(f)
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

func (runner *Linter) readLintResultFile(f *buildeventstream.File) (string, error) {
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
