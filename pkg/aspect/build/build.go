/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package build

import (
	"aspect.build/cli/pkg/pathutils"
	"context"
	"errors"
	"fmt"
	"github.com/spf13/viper"
	"os"
	"path/filepath"
	"time"

	"aspect.build/cli/pkg/aspect/build/bep"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/hooks"
	"aspect.build/cli/pkg/ioutils"
)

const (
	skipPromptKey = "build.skip_prompt"

	SpecifiedFolderOption = "All targets in a specified package"
	CurrentPackageOption  = "All targets within current package"
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
	bzl        bazel.Spawner
	besBackend bep.BESBackend
	hooks      *hooks.Hooks

	Behavior SelectRunner
	Remember PromptRunner
	Prefs    viper.Viper
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

// TODO: implement
// func GetAllInSpecifiedFolderPatern() (string, error) {

// }

// Returns a pattern for all targets within the current folder.
// If findNearestParentPackage is true, then this function returns a pattern for all targets
func GetAllInCurrentPackagePattern() (string, error) {
	workingDirectory, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("prompt failed: %w", err)
	}
	workspaceRoot := pathutils.FindWorkspaceRoot(workingDirectory)
	pkg := pathutils.FindNearestParentPackage(workingDirectory)
	if pkg == workspaceRoot {
		// Current directory is the WORKSPACE root
		return "//:all", nil
	}
	pkg, err = filepath.Rel(workspaceRoot, pkg)
	if err != nil {
		return "", fmt.Errorf("prompt failed: %w", err)
	}
	return "//" + pkg + ":all", nil
}

// Run runs the aspect build command, calling `bazel build` with a local Build
// Event Protocol backend used by Aspect plugins to subscribe to build events.
func (buildCmd *Build) Run(
	ctx context.Context,
	args []string,
	isInteractiveMode bool,
) (exitErr error) {
	skip := buildCmd.Prefs.GetBool(skipPromptKey)
	// TODO(f0rmiga): this is a hook for the build command and should be discussed
	// as part of the plugin design.
	target := ""
	if isInteractiveMode && !skip {
		_, chosen, err := buildCmd.Behavior.Run()

		if err != nil {
			return fmt.Errorf("prompt failed: %w", err)
		}

		switch chosen {
		case SpecifiedFolderOption:
			fmt.Fprint(buildCmd.Streams.Stdout, "Sorry, this is not implemented yet\n")
			return nil
		case CurrentPackageOption:
			target, err = GetAllInCurrentPackagePattern()
			if err != nil {
				return err
			}
			fmt.Fprint(buildCmd.Streams.Stdout, "Building "+target+"\n")
		case TargetPatternOption:
			fmt.Fprint(buildCmd.Streams.Stdout, "Sorry, this is not implemented yet\n")
			return nil
		}

	}
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
	cmd := []string{"build"}
	if target != "" {
		cmd = append(cmd, target)
	}
	cmd = append(cmd, besBackendFlag)
	exitCode, bazelErr := buildCmd.bzl.Spawn(append(cmd, args...))

	// Process the subscribers' errors before the Bazel one.
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
