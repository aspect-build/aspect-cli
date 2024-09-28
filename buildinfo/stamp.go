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

// Variables in this file will be replaced by the linker when Bazel is run with --stamp
// The time should be in format '2018-12-12 12:30:00 UTC'
// The GitStatus should be either "clean" or "dirty"
// Release will be a comma-separated string representation of any tags.
package buildinfo

// BuildTime is a string representation of when this binary was built.
var BuildTime = "an unknown time"

// HostName is the machine where this binary was built.
var HostName = "an unknown machine"

// GitCommit is the revision this binary was built from.
var GitCommit = "an unknown revision"

// GitStatus is whether the git workspace was clean.
var GitStatus = "unknown"

// Release is the revision number, if any.
var Release = "no release"

// OpenSource indicates if this is an Aspect CLI OSS build
var OpenSource = ""

func IsStamped() bool {
	return BuildTime != "{BUILD_TIMESTAMP}"
}
