/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package build

import (
	"context"
	"errors"
	"fmt"
	"time"

	"aspect.build/cli/pkg/aspect/build/bep"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/hooks"
	"aspect.build/cli/pkg/ioutils"
)

// Build represents the aspect build command.
type Build struct {
	ioutils.Streams
	bzl        bazel.Spawner
	besBackend bep.BESBackend
	hooks      *hooks.Hooks
}

// New creates a Build command.
func New(
	streams ioutils.Streams,
	bzl bazel.Spawner,
	besBackend bep.BESBackend,
	hooks *hooks.Hooks,
) *Build {
	return &Build{
		Streams:    streams,
		bzl:        bzl,
		besBackend: besBackend,
		hooks:      hooks,
	}
}

// Run runs the aspect build command, calling `bazel build` with a local Build
// Event Protocol backend used by Aspect plugins to subscribe to build events.
func (buildCmd *Build) Run(
	ctx context.Context,
	args []string,
	isInteractiveMode bool,
) (exitErr error) {
	// TODO(f0rmiga): this is a hook for the build command and should be discussed
	// as part of the plugin design.
	defer func() {
		errs := buildCmd.hooks.ExecutePostBuild(isInteractiveMode).Errors()
		if len(errs) > 0 {
			for _, err := range errs {
				fmt.Fprintf(buildCmd.Streams.Stderr, "Error: failed to run build command: %v\n", err)
			}
			var err *aspecterrors.ExitError
			if errors.As(exitErr, &err) {
				err.ExitCode = 1
			}
		}
	}()

	if err := buildCmd.besBackend.Setup(); err != nil {
		return fmt.Errorf("failed to run build command: %w", err)
	}
	ctx, cancel := context.WithTimeout(ctx, time.Second)
	defer cancel()
	if err := buildCmd.besBackend.ServeWait(ctx); err != nil {
		return fmt.Errorf("failed to run build command: %w", err)
	}
	defer buildCmd.besBackend.GracefulStop()

	besBackendFlag := fmt.Sprintf("--bes_backend=grpc://%s", buildCmd.besBackend.Addr())
	exitCode, bazelErr := buildCmd.bzl.Spawn(append([]string{"build", besBackendFlag}, args...))

	// Process the subscribers errors before the Bazel one.
	subscriberErrors := buildCmd.besBackend.Errors()
	if len(subscriberErrors) > 0 {
		for _, err := range subscriberErrors {
			fmt.Fprintf(buildCmd.Streams.Stderr, "Error: failed to run build command: %v\n", err)
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
