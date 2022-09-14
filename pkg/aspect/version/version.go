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
	"strings"

	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

type Version struct {
	ioutils.Streams

	BuildinfoRelease   string
	BuildinfoGitStatus string
	GNUFormat          bool
}

func New(streams ioutils.Streams) *Version {
	return &Version{
		Streams: streams,
	}
}

func (v *Version) Run(bzl bazel.Bazel) error {
	var versionBuilder strings.Builder
	if v.BuildinfoRelease != "" {
		versionBuilder.WriteString(v.BuildinfoRelease)
		if v.BuildinfoGitStatus != "clean" {
			versionBuilder.WriteString(" (with local changes)")
		}
	} else {
		versionBuilder.WriteString("unknown [not built with --stamp]")
	}
	version := versionBuilder.String()
	// Check if the --gnu_format flag is set, if that is the case,
	// the version is printed differently
	bazelCmd := []string{"version"}
	if v.GNUFormat {
		fmt.Fprintf(v.Stdout, "Aspect %s\n", version)
		// Propagate the flag
		bazelCmd = append(bazelCmd, "--gnu_format")
	} else {
		fmt.Fprintf(v.Stdout, "Aspect version: %s\n", version)
	}
	bzl.Spawn(bazelCmd, v.Streams)

	return nil
}
