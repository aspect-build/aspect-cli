/*
 * Copyright 2023 Aspect Build Systems, Inc.
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

/*
 * Copyright 2025 Aspect Build Systems, Inc.
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

package run

import (
	"context"
	"errors"
	"fmt"
	"log"
	"net"
	"os"
	"os/exec"
	"os/signal"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/aspect-build/aspect-cli/pkg/plugin/system/bep"
	"github.com/aspect-build/orion/common/ibp"
	watcher "github.com/aspect-build/orion/common/watch"
	"github.com/fatih/color"
	"github.com/spf13/cobra"
	"go.opentelemetry.io/otel"
	traceAttr "go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/trace"
)

// Run represents the aspect run command.
type Run struct {
	streams  ioutils.Streams
	hstreams ioutils.Streams
	bzl      bazel.Bazel

	tracer trace.Tracer
}

var watchConnectionTimeout = 1 * time.Second

func init() {
	timeoutEnv := os.Getenv("ASPECT_WATCH_CONNECTION_TIMEOUT_MS")
	if timeoutEnv != "" {
		timeout, err := strconv.Atoi(timeoutEnv)
		if err != nil {
			log.Fatalf("Invalid ASPECT_WATCH_CONNECTION_TIMEOUT_MS value (%v): %v", timeoutEnv, err)
		}
		watchConnectionTimeout = time.Duration(timeout) * time.Millisecond
	}
}

// New creates a Run command.
func New(
	streams ioutils.Streams,
	hstreams ioutils.Streams,
	bzl bazel.Bazel,
) *Run {
	return &Run{
		streams:  streams,
		hstreams: hstreams,
		bzl:      bzl,
		tracer:   otel.Tracer("aspect-run"),
	}
}

// Run runs the aspect run command, calling `bazel run` with a local Build
// Event Protocol backend used by Aspect plugins to subscribe to build events.
func (runner *Run) Run(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
	bazelCmd := []string{"run"}
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
	if !watch {
		err = runner.runCommand(ctx, bazelCmd, bzlCommandStreams)
	} else {
		err = runner.runWatch(ctx, bazelCmd, bzlCommandStreams)
	}

	// Check for subscriber errors
	subscriberErrors := bep.BESErrors(ctx)
	if len(subscriberErrors) > 0 {
		for _, serr := range subscriberErrors {
			fmt.Fprintf(runner.streams.Stderr, "Error: failed to run 'aspect run' command: %v\n", serr)
		}
		if err == nil {
			err = fmt.Errorf("%v BES subscriber error(s)", len(subscriberErrors))
		}
	}

	return err
}

func (runner *Run) runCommand(ctx context.Context, bazelCmd []string, bzlCommandStreams ioutils.Streams) error {
	ctx, t := runner.tracer.Start(ctx, "Run", trace.WithAttributes(
		traceAttr.StringSlice("command", bazelCmd),
	))
	defer t.End()

	return runner.bzl.RunCommand(bzlCommandStreams, nil, bazelCmd...)
}

func (runner *Run) runWatch(ctx context.Context, bazelCmd []string, bzlCommandStreams ioutils.Streams) error {
	fmt.Fprintf(
		runner.streams.Stderr,
		"%s Watching feature is experimental and may have breaking changes in the future.\n",
		color.YellowString("WARNING:"),
	)

	bazelInstall, err := runner.bzl.GetBazelInstallation()
	if err != nil {
		return fmt.Errorf("failed to get Bazel installation: %w", err)
	}

	changedetect, err := newChangeDetector(runner.bzl.WorkspaceRoot(), strings.HasPrefix(bazelInstall.Version, "7."))
	if err != nil {
		return fmt.Errorf("failed to created change detector: %w", err)
	}
	defer changedetect.Close()

	startScriptName := fmt.Sprintf("aspect-run-%v", os.Getpid())
	if runtime.GOOS == "windows" {
		startScriptName += ".bat"
	}

	startScript, err := os.CreateTemp(os.TempDir(), startScriptName)
	if err != nil {
		return err
	}
	defer func() {
		startScript.Close()
		os.Remove(startScript.Name())
	}()

	// Primary context to rule all async and background operations.
	// TODO: Cobras context seems to cancel too early. perhaps use that instead
	// of using our own signal?
	pcctx, cancel := context.WithCancel(context.Background())

	c := make(chan os.Signal, 1)
	signal.Notify(c, os.Interrupt, syscall.SIGTERM)
	go func() {
		<-c
		cancel()
	}()

	pcctx, t := runner.tracer.Start(pcctx, "Run.Watch", trace.WithAttributes(
		traceAttr.StringSlice("command", bazelCmd),
	))
	defer t.End()

	// The abazel protocol, potentially used as the incremental build tool.
	// Must initialize and start listening for connections before the initial bazel run command.
	// Start the incremental build service in case the process supports it and connects
	abazel := ibp.NewServer()

	// Start listening for a connection immediately.
	if err := abazel.Serve(pcctx); err != nil {
		return fmt.Errorf("failed to connect to aspect bazel protocol: %w", err)
	}

	// Close the watch protocol on complete, no matter what the status is
	defer abazel.Close()

	fmt.Printf("%s Listening on watch socket %s\n", color.GreenString("INFO:"), abazel.Address())

	createBazelScriptCmd := func(allowDiscard, trackChanges bool) (*exec.Cmd, error) {
		// Additional arguments for the bazel run command
		runCmdArgs := []string{}

		// ChangeDetector normally adds additional flags
		runCmdArgs = append(runCmdArgs, changedetect.bazelFlags(trackChanges)...)

		// --norun and generate a run script instead
		runCmdArgs = append(runCmdArgs, "--norun", "--script_path", startScript.Name())

		// --noallow_analysis_cache_discard except on the intial setup run
		if !allowDiscard {
			runCmdArgs = append(runCmdArgs, "--noallow_analysis_cache_discard")
		}

		return runner.bzl.MakeBazelCommand(pcctx, flags.AddFlagToCommand(bazelCmd, runCmdArgs...), bzlCommandStreams, nil, nil)
	}

	createRunCmd := func() *exec.Cmd {
		// Inherit the CLI environment variables
		env := os.Environ()[:]

		// Add the incremental build protocol(s) environment variables
		env = append(env, "IBAZEL_NOTIFY_CHANGES=y")
		if abazel != nil {
			env = append(env, abazel.Env()...)
		}

		startCmd := exec.CommandContext(pcctx, startScript.Name())
		startCmd.Stdin = bzlCommandStreams.Stdin
		startCmd.Stdout = bzlCommandStreams.Stdout
		startCmd.Stderr = bzlCommandStreams.Stderr
		startCmd.Env = env
		return startCmd
	}

	// Create and start the intial bazel command to build+inspect the run target
	initCmd, err := createBazelScriptCmd(true, false)
	if err != nil {
		return fmt.Errorf("failed to create initial bazel command: %w", err)
	}
	if err := initCmd.Run(); err != nil {
		return fmt.Errorf("initial bazel command failed: %w", err)
	}
	initCmd = nil

	// Detect the context of the run target after this initial build.
	if err := changedetect.detectContext(); err != nil {
		return fmt.Errorf("failed to detect context on init: %w", err)
	}

	// Start the workspace watcher
	w := watcher.NewWatchman(runner.bzl.WorkspaceRoot())
	if err := w.Start(); err != nil {
		return fmt.Errorf("failed to start the watcher: %w", err)
	}
	defer w.Close()

	// Since the Subscribe() method is blocking, we need to run a separate
	// goroutine to stop the watcher when we receive a signal to cancel the
	// process.
	go func() {
		<-pcctx.Done()
		w.Close()
	}()

	// The command to start the run target.
	startCmd := createRunCmd()

	// The incremental bazel protocol/tool to use going forward.
	var incrementalProtocol ibp.IncrementalBazel

	// If the target explicitly supports ibazel but NOT excplicitly supports incremental build protocol.
	if changedetect.supportsIBazelNotifyChanges() && !changedetect.explicitlySupportsIBP() {
		// Fallback to only using the legacy ibazel protocol.
		fmt.Printf("%s Fallback to legacy ibazel protocol\n", color.GreenString("INFO:"))

		// In order to support ibazel events we need to set the stdin to a pipe.
		// By default MakeBazelCommand sets it to bzlCommandStreams.stdin but we
		// want to control stdin depending on the watch mode.
		// In order to pipe stdin we need to set it to nil first and then call StdinPipe.
		startCmd.Stdin = nil
		runStdin, err := startCmd.StdinPipe()
		if err != nil {
			return fmt.Errorf("failed to create stdin pipe for ibazel: %w", err)
		}

		incrementalProtocol = &IBazelProtocol{
			stdin: runStdin,
		}
	} else {
		incrementalProtocol = abazel
	}

	// Close the incremental protocol when complete, no matter the protocol type.
	defer incrementalProtocol.Close()

	// Start the bazel command
	startErr := startCmd.Start()
	if startErr != nil {
		return fmt.Errorf("failed to start bazel command: %w", startErr)
	}

	// Give the watcher some time to start and open the connection before sending Init()
	if !incrementalProtocol.HasConnection() {
		// TODO: don't assume abazel is the only non-instant connection

		select {
		case <-pcctx.Done():
			fmt.Printf("%s Process cancelled before establishing connection: %v\n", color.RedString("ERROR:"), pcctx.Err())
			return pcctx.Err()
		case v := <-abazel.WaitForConnection():
			fmt.Printf("%s Received connection to %s using abazel v%v\n", color.GreenString("INFO:"), abazel.Address(), v)
		case <-time.After(watchConnectionTimeout):
			fmt.Printf("%s Timeout waiting for watch protocol connection.\n", color.YellowString("WARNING:"))
			break
		}
	}

	// Abandon the incrmental protocol if the target has not responded
	if !incrementalProtocol.HasConnection() {
		fmt.Printf("%s No watch protocol connection established. Fallback to restart.\n", color.YellowString("WARNING:"))

		if changedetect.explicitlySupportsIBP() {
			fmt.Printf("%s target explicitly supports incremental build protocol but did not connect.\n", color.RedString("WARNING:"))
		}

		go abazel.Close()
		abazel = nil

		incrementalProtocol = &RestartBazelProtocol{
			createRunCmd: createRunCmd,
			runCmd:       startCmd,
		}
	}

	// Init() with the full runfiles list
	initRunfiles, initRunfielsErr := changedetect.loadFullSourceInfo()
	if initRunfielsErr != nil {
		return fmt.Errorf("failed to load initial runfiles: %w", initRunfielsErr)
	}
	initErr := incrementalProtocol.Init(initRunfiles)
	if initErr != nil {
		return fmt.Errorf("failed to initialize watch protocol: %w", initErr)
	}

	// Send an 'Exit' message to the child process when the context completes in case
	// the context was cancelled due to the cli being shutdown.
	go func() {
		<-pcctx.Done()

		// If a connection still exists to the incremental protocol, send an Exit message and
		// hope for a graceful shutdown. Ignore any errors as the process may already be in the
		// process of shutting down.
		if incrementalProtocol.HasConnection() {
			incrementalProtocol.Exit(err)
		}

		// Terminate the process if it is still running.
		terminate(startCmd.Process)
	}()

	pcctx, st := runner.tracer.Start(pcctx, "Run.Subscribe")
	defer st.End()

	// Subscribe to further changes
	for cs, err := range w.Subscribe(pcctx, "aspect-run-watch") {
		if err != nil {
			// Break the subscribe iteration if the context is done or if the watcher is closed.
			if errors.Is(err, context.Canceled) || errors.Is(err, net.ErrClosed) {
				break
			}

			return fmt.Errorf("failed to get next event: %w", err)
		}

		_, st := runner.tracer.Start(pcctx, "Run.Subscribe.Trigger")

		// Enter into the build state to discard supirious changes caused by Bazel reading the
		// inputs which leads to their atime to change.
		if err := w.StateEnter("aspect-run-watch"); err != nil {
			return fmt.Errorf("failed to enter build state: %w", err)
		}

		// The command to detect changes in the run target.
		detectCmd, err := createBazelScriptCmd(false, true)
		if err != nil {
			return fmt.Errorf("failed to create bazel detect command: %w", err)
		}

		// Something has changed, but we have no idea if it affects our target.
		// Normally we'd want to perform a cquery to determine if it affects but
		// that is too costly especially in larger monorepos. So instead we rebuild
		// the target with --execution_log_json_file and determine if it ran any
		// actions.
		//
		// TODO: delay the command stdout and do not output on quick noops
		incBuildErr := detectCmd.Run()

		dtErr := changedetect.detectChanges(cs.Paths)
		if dtErr != nil {
			return fmt.Errorf("failed to detect changes: %w", dtErr)
		}

		if incBuildErr != nil {
			// The incremental build failed.
			// Assume a temporary compilation error, assume an appopriate error message was outputted by the run command.
			// Output a basic warning and resume waiting for changes.
			fmt.Printf("%s incremental bazel build command failed: %v\n", color.YellowString("WARNING:"), incBuildErr)
		} else if changes := changedetect.cycleChanges(); len(changes) > 0 {
			// For now just rerun the target, beware that RunCommand does not yield until
			// the subprocess exists.
			fmt.Printf("%s Found %d changes, rebuilding the target.\n", color.GreenString("INFO:"), len(changes))

			if err := incrementalProtocol.Cycle(changes); err != nil {
				return fmt.Errorf("failed to report cycle events: %w", err)
			}

			// TODO: if we want to support ibazel livereload then we need to report changes.
		} else {
			fmt.Printf("%s Target is up-to-date.\n", color.GreenString("INFO:"))
		}

		// Leave the build state and fast forward the subscription clock.
		if err := w.StateLeave("aspect-run-watch"); err != nil {
			return fmt.Errorf("failed to enter build state: %w", err)
		}

		st.End()
	}

	return nil
}
