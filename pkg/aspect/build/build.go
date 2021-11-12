/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package build

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/spf13/viper"

	buildeventstream "aspect.build/cli/bazel/buildeventstream/proto"
	"aspect.build/cli/pkg/aspect/build/bep"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/hooks"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/pathutils"
)

const (
	skipPromptKey = "build.skip_prompt"

	SpecifiedFolderOption = "All targets in a specified folder (path relative to your WORKSPACE)"
	CurrentFolderOption   = "All targets in current folder"
	TargetPatternOption   = "Specific target patterns"

	RememberLine1 = "Remember this choice and skip the prompt in the future"
)

type SelectRunner interface {
	Run() (int, string, error)
}

type PromptRunner interface {
	Run() (string, error)
}

// Build represents the aspect build command.
type Build struct {
	ioutils.Streams
	bzl               bazel.Spawner
	isInteractiveMode bool

	Behavior SelectRunner
	Remember PromptRunner
	Prefs    viper.Viper

	besBackend bep.BESBackend
	hooks      *hooks.Hooks
}

// New creates a Build command.
func New(
	streams ioutils.Streams,
	bzl bazel.Spawner,
	isInteractiveMode bool,
	besBackend bep.BESBackend,
	hooks *hooks.Hooks,
) *Build {
	return &Build{
		Streams:           streams,
		bzl:               bzl,
		isInteractiveMode: isInteractiveMode,
		besBackend:        besBackend,
		hooks:             hooks,
	}
}

// Returns a Bazel pattern for all targets within the current folder
func GetAllInCurrentFolderPattern() (string, error) {
	workingDirectory, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("prompt failed: %w", err)
	}
	workspaceRoot := pathutils.FindWorkspaceRoot(workingDirectory)
	if workspaceRoot == "" {
		return "", fmt.Errorf("prompt failed: %w", "Current working directory not within a Bazel workspace!")
	}
	target, err := filepath.Rel(workspaceRoot, workingDirectory)
	if err != nil {
		return "", fmt.Errorf("prompt failed: %w", err)
	}
	if target == "." {
		return "//...", nil
	}
	return "//" + target + "/...", nil
}

// Run runs the aspect build command, calling `bazel build` with a local Build
// Event Protocol backend used by Aspect plugins to subscribe to build events.
func (b *Build) Run(ctx context.Context, args []string) (exitErr error) {
	skip := b.Prefs.GetBool(skipPromptKey)
	target := ""
	if b.isInteractiveMode && !skip {
		_, chosen, err := b.Behavior.Run()

		if err != nil {
			return fmt.Errorf("prompt failed: %w", err)
		}

		switch chosen {
		case SpecifiedFolderOption:
			fmt.Fprint(b.Streams.Stdout, "Sorry, this is not implemented yet\n")
			return nil
		case CurrentFolderOption:
			target, err = GetAllInCurrentFolderPattern()
			if err != nil {
				return err
			}
			fmt.Fprint(b.Streams.Stdout, "Building "+target+"\n")
		case TargetPatternOption:
			fmt.Fprint(b.Streams.Stdout, "Sorry, this is not implemented yet\n")
			return nil
		}

	}

	// TODO(f0rmiga): this is a hook for the build command and should be discussed
	// as part of the plugin design.
	defer func() {
		errs := b.hooks.ExecutePostBuild().Errors()
		if len(errs) > 0 {
			for _, err := range errs {
				fmt.Fprintf(b.Streams.Stderr, "Error: failed to run build command: %v\n", err)
			}
			var err *aspecterrors.ExitError
			if errors.As(exitErr, &err) {
				err.ExitCode = 1
			}
		}
	}()

	if err := b.besBackend.Setup(); err != nil {
		return fmt.Errorf("failed to run build command: %w", err)
	}
	ctx, cancel := context.WithTimeout(ctx, time.Second)
	defer cancel()
	if err := b.besBackend.ServeWait(ctx); err != nil {
		return fmt.Errorf("failed to run build command: %w", err)
	}
	defer b.besBackend.GracefulStop()

	besBackendFlag := fmt.Sprintf("--bes_backend=grpc://%s", b.besBackend.Addr())
	cmd := []string{"build"}
	if target != "" {
		cmd = append(cmd, target)
	}
	cmd = append(cmd, besBackendFlag)
	exitCode, bazelErr := b.bzl.Spawn(append(cmd, args...))

	// Process the subscribers' errors before the Bazel one.
	subscriberErrors := b.besBackend.Errors()
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

// Plugin defines only the methods for the build command.
type Plugin interface {
	// BEPEventsSubscriber is used to verify whether an Aspect plugin registers
	// itself to receive the Build Event Protocol events.
	BEPEventCallback(event *buildeventstream.BuildEvent) error
	// TODO(f0rmiga): test the build hooks after implementing the plugin system.
	PostBuildHook() error
}
