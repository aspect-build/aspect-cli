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

package fetch

import (
	"context"

	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/spf13/cobra"
)

type Fetch struct {
	streams  ioutils.Streams
	hstreams ioutils.Streams
	bzl      bazel.Bazel
}

func New(streams ioutils.Streams, hstreams ioutils.Streams, bzl bazel.Bazel) *Fetch {
	return &Fetch{
		streams:  streams,
		hstreams: hstreams,
		bzl:      bzl,
	}
}

func (runner *Fetch) Run(ctx context.Context, cmd *cobra.Command, args []string) error {
	bazelCmd := []string{"fetch"}
	bazelCmd = append(bazelCmd, args...)

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

	return runner.bzl.RunCommand(bzlCommandStreams, nil, bazelCmd...)
}
