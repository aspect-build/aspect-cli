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
	"context"
	"errors"
	"fmt"
	"net"
	"os"
	"os/signal"
	"syscall"

	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/aspect-build/aspect-cli/pkg/plugin/system/bep"
	"github.com/aspect-build/aspect-cli/pkg/watch"
	"github.com/fatih/color"
	"github.com/spf13/cobra"
)

// Build represents the aspect build command.
type Build struct {
	streams  ioutils.Streams
	hstreams ioutils.Streams
	bzl      bazel.Bazel
}

// New creates a Build command.
func New(
	streams ioutils.Streams,
	hstreams ioutils.Streams,
	bzl bazel.Bazel,
) *Build {
	return &Build{
		streams:  streams,
		hstreams: hstreams,
		bzl:      bzl,
	}
}

// Run runs the aspect build command, calling `bazel build` with a local Build
// Event Protocol backend used by Aspect plugins to subscribe to build events.
func (runner *Build) Run(ctx context.Context, cmd *cobra.Command, args []string) error {
	bazelCmd := []string{"build"}
	watch, args := flags.RemoveFlag(args, "--watch")
	bazelCmd = append(bazelCmd, args...)

	// Currently Bazel only supports a single --bes_backend so adding ours after
	// any user supplied value will result in our bes_backend taking precedence.
	// There is a very old & stale issue to add support for multiple BES
	// backends https://github.com/bazelbuild/bazel/issues/10908. In the future,
	// we could build this support into the Aspect CLI and post on that issue
	// that using the Aspect CLI resolves it.
	if bep.HasBESBackend(ctx) {
		besBackend := bep.BESBackendFromContext(ctx)
		besBackendFlag := fmt.Sprintf("--bes_backend=%s", besBackend.Addr())
		bazelCmd = flags.AddFlagToCommand(bazelCmd, besBackendFlag)
	}

	bzlCommandStreams := runner.streams
	if cmd != nil {
		hints, err := cmd.Root().PersistentFlags().GetBool(flags.AspectHintsFlagName)
		if err != nil {
			return err
		}
		if hints {
			bzlCommandStreams = runner.hstreams
		}
	}

	var err error
	if watch {
		// TODO: reduce duplication with test/run--watch

		fmt.Fprintf(
			runner.streams.Stderr,
			"%s Watching feature is experimental and may have breaking changes in the future.\n",
			color.YellowString("WARNING:"),
		)

		pcctx, cancel := context.WithCancel(context.Background())

		c := make(chan os.Signal, 1)
		signal.Notify(c, os.Interrupt, syscall.SIGTERM)
		go func() {
			<-c
			cancel()
		}()

		err = runner.buildWatch(pcctx, bazelCmd, bzlCommandStreams)
	} else {
		err = runner.bzl.RunCommand(bzlCommandStreams, nil, bazelCmd...)
	}

	// Check for subscriber errors
	subscriberErrors := bep.BESErrors(ctx)
	if len(subscriberErrors) > 0 {
		for _, err := range subscriberErrors {
			fmt.Fprintf(runner.streams.Stderr, "Error: failed to run build command: %v\n", err)
		}
		if err == nil {
			err = fmt.Errorf("%v BES subscriber error(s)", len(subscriberErrors))
		}
	}

	return err
}

func (runner *Build) buildWatch(ctx context.Context, bazelCmd []string, streams ioutils.Streams) error {
	// TODO: reduce duplication with test/run--watch

	// Start the workspace watcher
	w := watch.NewWatchman(runner.bzl.WorkspaceRoot())
	if err := w.Start(); err != nil {
		return fmt.Errorf("failed to start the watcher: %w", err)
	}
	defer w.Close()

	// Since the Subscribe() method is blocking, we need to run a separate
	// goroutine to stop the watcher when we receive a signal to cancel the
	// process.
	go func() {
		<-ctx.Done()
		w.Close()
	}()

	err := runner.bzl.RunCommand(streams, nil, bazelCmd...)
	if err != nil {
		fmt.Printf("Initial Build Failed: %v", err)
	}

	for _, err := range w.Subscribe(ctx, "aspect-build-watch") {
		if err != nil {
			// Break the subscribe iteration if the context is done or if the watcher is closed.
			if errors.Is(err, context.Canceled) || errors.Is(err, net.ErrClosed) {
				break
			}

			return fmt.Errorf("failed to get next event: %w", err)
		}

		// Enter into the build state to discard supirious changes caused by Bazel reading the
		// inputs which leads to their atime to change.
		if err := w.StateEnter("aspect-build-watch"); err != nil {
			return fmt.Errorf("failed to enter build state: %w", err)
		}

		err := runner.bzl.RunCommand(streams, nil, bazelCmd...)
		if err != nil {
			fmt.Printf("Incremental Build Failed: %v", err)
		}

		// Leave the build state and fast forward the subscription clock.
		if err := w.StateLeave("aspect-build-watch"); err != nil {
			return fmt.Errorf("failed to exit build state: %w", err)
		}
	}

	return nil
}
