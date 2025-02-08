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

package test

import (
	"context"
	"fmt"

	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/aspect-build/aspect-cli/pkg/plugin/system/bep"
	"github.com/spf13/cobra"
)

type Test struct {
	streams  ioutils.Streams
	hstreams ioutils.Streams
	bzl      bazel.Bazel
}

func New(streams ioutils.Streams, hstreams ioutils.Streams, bzl bazel.Bazel) *Test {
	return &Test{
		streams:  streams,
		hstreams: hstreams,
		bzl:      bzl,
	}
}

func (runner *Test) Run(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
	bazelCmd := []string{"test"}
	bazelCmd = append(bazelCmd, args...)

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
	}

	bzlCommandStreams := runner.streams
	if cmd != nil {
		hints, err := cmd.Root().PersistentFlags().GetBool(flags.AspectHintsFlagName)
		if err != nil {
			return err
		}
		if hints {
			bzlCommandStreams = runner.hstreams
		}
	}

	err := runner.bzl.RunCommand(bzlCommandStreams, nil, bazelCmd...)

	// Check for subscriber errors
	subscriberErrors := bep.BESErrors(ctx)
	if len(subscriberErrors) > 0 {
		for _, err := range subscriberErrors {
			fmt.Fprintf(runner.streams.Stderr, "Error: failed to run test command: %v\n", err)
		}
		if err == nil {
			err = fmt.Errorf("%v BES subscriber error(s)", len(subscriberErrors))
		}
	}

	return err
}
