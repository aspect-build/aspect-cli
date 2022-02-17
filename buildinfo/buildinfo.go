/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
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

func IsStamped() bool {
	return BuildTime != "{BUILD_TIME}"
}
