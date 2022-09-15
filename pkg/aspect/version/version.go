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
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

type Version struct {
	ioutils.Streams

	BuildInfo buildinfo.BuildInfo
	GNUFormat bool
}

func New(streams ioutils.Streams) *Version {
	return &Version{
		Streams: streams,
	}
}

func (v *Version) Run(bzl bazel.Bazel) error {
	// Write the version
	format := buildinfo.ConventionalFormat
	if v.GNUFormat {
		format = buildinfo.GNUFormat
	}
	version := v.BuildInfo.CommandVersion("Aspect", format)
	if _, err := fmt.Fprintln(v.Stdout, version); err != nil {
		return err
	}

	// Check if the --gnu_format flag is set, if that is the case,
	// the version is printed differently
	bazelCmd := []string{"version"}
	if v.GNUFormat {
		// Propagate the flag
		bazelCmd = append(bazelCmd, "--gnu_format")
	}
	bzl.Spawn(bazelCmd, v.Streams)

	return nil
}
