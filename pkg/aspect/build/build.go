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

package build

import (
	"fmt"

	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system/bep"
)

// Build represents the aspect build command.
type Build struct {
	ioutils.Streams
	bzl bazel.Bazel
}

// New creates a Build command.
func New(
	streams ioutils.Streams,
	bzl bazel.Bazel,
) *Build {
	return &Build{
		Streams: streams,
		bzl:     bzl,
	}
}

// Run runs the aspect build command, calling `bazel build` with a local Build
// Event Protocol backend used by Aspect plugins to subscribe to build events.
func (b *Build) Run(args []string, besBackend bep.BESBackend) (exitErr error) {
	besBackendFlag := fmt.Sprintf("--bes_backend=%s", besBackend.Addr())
	exitCode, bazelErr := b.bzl.RunCommand(append([]string{"build", besBackendFlag}, args...), b.Streams)

	// Process the subscribers errors before the Bazel one.
	subscriberErrors := besBackend.Errors()
	if len(subscriberErrors) > 0 {
		for _, err := range subscriberErrors {
			fmt.Fprintf(b.Streams.Stderr, "Error: failed to run build command: %v\n", err)
		}
		exitCode = 1
	}

	if exitCode != 0 {
		err := &aspecterrors.ExitError{ExitCode: exitCode}
		if bazelErr != nil {
			err.Err = bazelErr
		}
		return err
	}

	return nil
}
