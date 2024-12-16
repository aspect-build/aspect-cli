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

package version

import (
	"context"
	"fmt"

	"github.com/aspect-build/aspect-cli/buildinfo"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/spf13/cobra"
)

type Version struct {
	ioutils.Streams
	bzl       bazel.Bazel
	BuildInfo buildinfo.BuildInfo
}

func New(streams ioutils.Streams, bzl bazel.Bazel) *Version {
	return &Version{
		Streams:   streams,
		bzl:       bzl,
		BuildInfo: *buildinfo.Current(),
	}
}

func (runner *Version) Run(ctx context.Context, cmd *cobra.Command, args []string) error {
	// Determine the format
	format := buildinfo.ConventionalFormat
	gnuFormat, err := cmd.Flags().GetBool("gnu_format")
	if err != nil {
		return fmt.Errorf("failed to get value of --gnu_format flag: %w", err)
	}
	if gnuFormat {
		format = buildinfo.GNUFormat
	}

	// Write the version
	version := runner.BuildInfo.CommandVersion(format)
	if _, err := fmt.Fprintln(runner.Stdout, version); err != nil {
		return err
	}

	bazelCmd := []string{"version"}
	bazelCmd = append(bazelCmd, args...)
	return runner.bzl.RunCommand(runner.Streams, nil, bazelCmd...)
}
