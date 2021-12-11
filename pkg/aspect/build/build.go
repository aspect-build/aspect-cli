/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
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
	besBackendFlag := fmt.Sprintf("--bes_backend=grpc://%s", besBackend.Addr())
	exitCode, bazelErr := b.bzl.Spawn(append([]string{"build", besBackendFlag}, args...))

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
