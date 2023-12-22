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

	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"github.com/spf13/cobra"
)

type Fetch struct {
	ioutils.Streams
	bzl bazel.Bazel
}

func New(streams ioutils.Streams, bzl bazel.Bazel) *Fetch {
	return &Fetch{
		Streams: streams,
		bzl:     bzl,
	}
}

func (runner *Fetch) Run(ctx context.Context, _ *cobra.Command, args []string) error {
	bazelCmd := []string{"fetch"}
	bazelCmd = append(bazelCmd, args...)
	return runner.bzl.RunCommand(runner.Streams, nil, bazelCmd...)
}
