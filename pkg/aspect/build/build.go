/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package build

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/spf13/viper"

	"aspect.build/cli/pkg/aspect/build/bep"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/hooks"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/pathutils"
)

const (
	skipPromptKey = "build.skip_prompt"

	CommonTargetsOption    = "Common targets defined in a specified package (path relative to your workspace root)"
	CurrentPackageOption   = "All targets within current package"
	SpecifiedPackageOption = "All targets in a specified package (path relative to your workspace root)"
	TargetPatternOption    = "Specific target patterns"

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
	targets := []string{}
	if isInteractiveMode && !skip {
		_, chosen, err := buildCmd.Behavior.Run()

		if err != nil {
			return fmt.Errorf("prompt failed: %w", err)
		}

		switch chosen {
		case CommonTargetsOption:
			fmt.Print("Enter the workspace-relative path to the package:\n")
			inputPath, err := ioutils.ReadLine()
			if err != nil {
				return err
			}
			pkg, err := pathutils.FindNearestParentPackage(inputPath)
			if err != nil {
				return err
			}
			targets, err = ioutils.ReadCommonTargets(pkg)
			if err != nil {
				return err
			}
		case CurrentPackageOption:
			target, err := pathutils.GetAllInCurrentPackagePattern()
			if err != nil {
				return err
			}
			targets = append(targets, target)
		case SpecifiedPackageOption:
			fmt.Print("Enter the workspace-relative path to the package:\n")
			path, err := ioutils.ReadLine()
			if err != nil {
				return err
			}
			target, err := pathutils.GetAllInSpecifiedPackagePattern(path)
			targets = append(targets, target)
			if err != nil {
				return err
			}
		case TargetPatternOption:
			fmt.Print("Enter the target patterns you want to build, separated by spaces:\n")
			targetString, err := ioutils.ReadLine()
			targets = strings.Split(targetString, " ")
			if err != nil {
				return err
			}
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
	if len(targets) > 0 {
		for _, target := range targets {
			cmd = append(cmd, target)
		}
		fmt.Fprint(buildCmd.Streams.Stdout, "Building " + strings.Join(targets, " ") + "\n")
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
