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
	"fmt"

	"aspect.build/cli/buildinfo"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"github.com/spf13/cobra"
)

type Version struct {
	ioutils.Streams

	BuildInfo buildinfo.BuildInfo
}

func New(streams ioutils.Streams) *Version {
	return &Version{
		Streams: streams,
	}
}

func (runner *Version) Run(cmd *cobra.Command, bzl bazel.Bazel, args []string) error {
	// Determine the format
	format := buildinfo.ConventionalFormat
	if cmd != nil {
		gnuFormat, err := cmd.Flags().GetBool("gnu_format")
		if err != nil {
			return fmt.Errorf("failed to get value of --gnu_format flag: %w", err)
		}
		if gnuFormat {
			format = buildinfo.GNUFormat
		}
	}

	// Write the version
	version := runner.BuildInfo.CommandVersion(format)
	if _, err := fmt.Fprintln(runner.Stdout, version); err != nil {
		return err
	}

	// If we do not have a Bazel workspace, do not bother trying to get additional version
	// information.
	if bzl == nil {
		return nil
	}

	bazelCmd := []string{"version"}
	bazelCmd = append(bazelCmd, args...)
	if exitCode, err := bzl.RunCommand(runner.Streams, bazelCmd...); exitCode != 0 {
		err = &aspecterrors.ExitError{
			Err:      err,
			ExitCode: exitCode,
		}
		return err
	}

	return nil
}
