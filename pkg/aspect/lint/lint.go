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
	"path/filepath"
	"strings"

	"github.com/fatih/color"

	"aspect.build/cli/bazel/buildeventstream"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system/bep"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"
)

type Linter struct {
	ioutils.Streams
	bzl           bazel.Bazel
	bazelBin      string
	executionRoot string
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
		runner.executionRoot = event.GetWorkspaceInfo().GetLocalExecRoot()

	case *buildeventstream.BuildEvent_Configuration:
		runner.bazelBin = event.GetConfiguration().GetMakeVariable()["BINDIR"]

	// TODO: are we printing too much? Don't we need to filter on the "report" output group?
	case *buildeventstream.BuildEvent_NamedSetOfFiles:
		for _, f := range event.GetNamedSetOfFiles().GetFiles() {
			// TODO: what about Build Without the Bytes
			if strings.HasPrefix(f.GetUri(), "file://") {
				localFilePath := strings.TrimPrefix(f.GetUri(), "file://")
				lintResultBuf, err := os.ReadFile(localFilePath)
				if err != nil {
					err = &aspecterrors.ExitError{
						Err:      err,
						ExitCode: 1,
					}
					return err
				}

				lineResult := strings.TrimSpace(string(lintResultBuf))
				if len(lineResult) > 0 {
					relpath, _ := filepath.Rel(filepath.Join(runner.executionRoot, runner.bazelBin), localFilePath)
					color.New(color.FgYellow).Fprintf(runner.Streams.Stdout, "From %s:\n", relpath)
					fmt.Fprintln(runner.Streams.Stdout, lineResult)
					fmt.Fprintln(runner.Streams.Stdout, "")
				}
			}
		}
	}
	return nil
}
