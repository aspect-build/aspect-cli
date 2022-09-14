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

package buildinfo

import "strings"

const (
	// Git status
	CleanGitStatus = "clean"

	// Version related constants
	NotCleanVersionSuffix = " (with local changes)"
	NoReleaseVersion      = "unknown [not built with --stamp]"
)

type BuildInfo struct {
	BuildTime string
	HostName  string
	GitCommit string
	GitStatus string
	Release   string
}

func New(
	buildTime string,
	hostName string,
	gitCommit string,
	gitStatus string,
	release string,
) *BuildInfo {
	return &BuildInfo{
		BuildTime: buildTime,
		HostName:  hostName,
		GitCommit: gitCommit,
		GitStatus: gitStatus,
		Release:   release,
	}
}

func Current() *BuildInfo {
	return New(
		BuildTime,
		HostName,
		GitCommit,
		GitStatus,
		Release,
	)
}

func (bi BuildInfo) HasRelease() bool {
	// TODO(chuck): Add check for "no release"
	return bi.Release != ""
}

func (bi BuildInfo) IsClean() bool {
	return bi.GitStatus == CleanGitStatus
}

func (bi BuildInfo) Version() string {
	var versionBuilder strings.Builder
	if bi.HasRelease() {
		versionBuilder.WriteString(bi.Release)
		if !bi.IsClean() {
			versionBuilder.WriteString(NotCleanVersionSuffix)
		}
	} else {
		versionBuilder.WriteString(NoReleaseVersion)
	}
	return versionBuilder.String()
}
