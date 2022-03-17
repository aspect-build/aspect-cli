/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package run

import (
	"fmt"

	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system/bep"
)

// Run represents the aspect run command.
type Run struct {
	ioutils.Streams
	bzl bazel.Bazel
}

// New creates a Run command.
func New(
	streams ioutils.Streams,
	bzl bazel.Bazel,
) *Run {
	return &Run{
		Streams: streams,
		bzl:     bzl,
	}
}

// Run runs the aspect run command, calling `bazel run` with a local Build
// Event Protocol backend used by Aspect plugins to subscribe to build events.
func (cmd *Run) Run(args []string, besBackend bep.BESBackend) (exitErr error) {
	besBackendFlag := fmt.Sprintf("--bes_backend=%s", besBackend.Addr())
	exitCode, bazelErr := cmd.bzl.Spawn(append([]string{"run", besBackendFlag}, args...))

	// Process the subscribers errors before the Bazel one.
	subscriberErrors := besBackend.Errors()
	if len(subscriberErrors) > 0 {
		for _, err := range subscriberErrors {
			fmt.Fprintf(cmd.Streams.Stderr, "Error: failed to run 'aspect run' command: %v\n", err)
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
