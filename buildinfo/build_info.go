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

import (
	"fmt"
	"strings"
)

const (
	// Git status
	CleanGitStatus = "clean"

	// Release values
	PreStampRelease = "no release"

	// Version constants
	NotCleanVersionSuffix = " (with local changes)"
	NoReleaseVersion      = "unknown [not built with --stamp]"
)

type VersionFormat int

const (
	// VersionFormat
	ConventionalFormat VersionFormat = iota
	GNUFormat
)

type BuildInfo struct {
	BuildTime  string
	HostName   string
	GitCommit  string
	GitStatus  string
	Release    string
	OpenSource bool
}

func New(
	buildTime string,
	hostName string,
	gitCommit string,
	gitStatus string,
	release string,
	oss bool,
) *BuildInfo {
	return &BuildInfo{
		BuildTime:  buildTime,
		HostName:   hostName,
		GitCommit:  gitCommit,
		GitStatus:  gitStatus,
		Release:    release,
		OpenSource: oss,
	}
}

func Current() *BuildInfo {
	return New(
		BuildTime,
		HostName,
		GitCommit,
		GitStatus,
		Release,
		OpenSource != "",
	)
}

func (bi BuildInfo) HasRelease() bool {
	return bi.Release != "" && bi.Release != PreStampRelease
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

func (bi BuildInfo) Name() string {
	if bi.OpenSource {
		return "Aspect CLI OSS"
	} else {
		return "Aspect CLI"
	}
}

func (bi BuildInfo) GnuName() string {
	if bi.OpenSource {
		return "aspect oss"
	} else {
		return "aspect"
	}
}

func (bi BuildInfo) CommandVersion(format VersionFormat) string {
	switch format {
	case GNUFormat:
		return fmt.Sprintf("%s %s", bi.GnuName(), bi.Version())
	case ConventionalFormat:
		// Conventional is the default case
		fallthrough
	default:
		// Use the Conventional format, if not recognized
		return fmt.Sprintf("%s version: %s", bi.Name(), bi.Version())
	}
}
