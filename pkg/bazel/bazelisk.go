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

package bazel

import (
	"bufio"
	"context"
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/bazelbuild/bazelisk/core"
	"github.com/bazelbuild/bazelisk/httputil"
	"github.com/bazelbuild/bazelisk/platforms"
	"github.com/mitchellh/go-homedir"

	"github.com/aspect-build/aspect-cli/buildinfo"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/config"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/aspect-build/aspect-cli/pkg/ioutils/cache"
)

const (
	aspectReentrantEnv = "ASPECT_REENTRANT"
	useBazelVersionEnv = "USE_BAZEL_VERSION"
)

type Bazelisk struct {
	workspaceRoot string

	allowReenter bool

	// Set to true in getBazelVersion() if this aspect binary is not the user's configured
	// version and should re-enter another aspect binary of a different version
	AspectShouldReenter bool
}

func NewBazelisk(workspaceRoot string, allowReenter bool) *Bazelisk {
	return &Bazelisk{
		workspaceRoot: workspaceRoot,
		allowReenter:  allowReenter,
	}
}

func (bazelisk *Bazelisk) GetBazelPath(repos *core.Repositories) (string, error) {
	httputil.UserAgent = bazelisk.getUserAgent()

	bazeliskHome := bazelisk.GetEnvOrConfig("BAZELISK_HOME")
	if len(bazeliskHome) == 0 {
		userCacheDir, err := cache.UserCacheDir()
		if err != nil {
			return "", err
		}

		bazeliskHome = filepath.Join(userCacheDir, "bazelisk")
	}

	err := os.MkdirAll(bazeliskHome, 0755)
	if err != nil {
		return "", fmt.Errorf("could not create directory %s: %v", bazeliskHome, err)
	}

	bazelVersionString, baseUrl, err := bazelisk.getBazelVersion()
	if err != nil {
		return "", fmt.Errorf("could not get Bazel version: %v", err)
	}

	if bazelisk.AspectShouldReenter {
		// Work-around for re-entering older versions of the Aspect CLI that didn't handle
		// env boostrap correctly.
		scrubEnvOfBazeliskAspectBootstrap()
	}

	bazelPath, err := homedir.Expand(bazelVersionString)
	if err != nil {
		return "", fmt.Errorf("could not expand home directory in path: %v", err)
	}

	// If we aren't using a local Bazel binary, we'll have to parse the version string and
	// download the version that the user wants.
	if !filepath.IsAbs(bazelPath) {
		bazelPath, err = downloadBazel(bazelVersionString, baseUrl, bazeliskHome, repos)
		if err != nil {
			return "", fmt.Errorf("could not download Bazel: %v", err)
		}
	} else {
		baseDirectory := filepath.Join(bazeliskHome, "local")
		bazelPath, err = linkLocalBazel(baseDirectory, bazelPath)
		if err != nil {
			return "", fmt.Errorf("could not link local Bazel: %v", err)
		}
	}

	return bazelPath, nil
}

// Run runs the main Bazelisk logic for the given arguments and Bazel repositories.
func (bazelisk *Bazelisk) Run(args []string, repos *core.Repositories, streams ioutils.Streams, env []string, wd *string) error {
	bazelPath, err := bazelisk.GetBazelPath(repos)
	if err != nil {
		return fmt.Errorf("could not get path to Bazel: %v", err)
	}

	exitCode, err := bazelisk.runBazel(bazelPath, args, streams, env, wd)
	if err != nil {
		return fmt.Errorf("could not run Bazel: %v", err)
	}
	if exitCode != 0 {
		// Just bubble up the exit code so the Aspect CLI exits with the same code; don't specify any error
		// message since Bazel should have already printed the error to stderr if appropriate and we don't
		// want to print any additional error messages to stderr.
		return &aspecterrors.ExitError{
			Err:      nil,
			ExitCode: exitCode,
		}
	}
	return nil
}

type aspectRuntimeInfo struct {
	Reentrant bool
	Version   string
	DevBuild  bool
	BaseUrl   string
}

type bazeliskVersionConfig struct {
	UseBazelVersion string
	BazeliskBaseUrl string
}

func isBazeliskAspectBootstrap(bazeliskConfig *bazeliskVersionConfig) bool {
	if strings.HasPrefix(bazeliskConfig.UseBazelVersion, "aspect-cli/") {
		// aspect-cli/ org is reserved for future use so that we can bootstrap Aspect CLI with
		// bazelisk without a BAZELISK_BASE_URL from the releases in this repository
		// https://github.com/aspect-cli/bazel; a fix in bazelisk is required for this to work,
		// however.
		return true
	}
	if strings.HasPrefix(bazeliskConfig.UseBazelVersion, "aspect/") {
		// aspect/ org is a special case incase a user has a fork of the aspect-cli repo and has a
		// custom BAZELISK_BASE_URL we can't detect; we generally have it set in all of our
		// .bazeliskrc examples as best practice even tho it is not strictly needed if you set the
		// BAZELISK_BASE_URL to https://github.com/aspect-build/aspect-cli/releases/download.
		return true
	}
	if bazeliskConfig.BazeliskBaseUrl == "https://github.com/aspect-build/aspect-cli/releases/download" {
		// GitHub aspect-cli OSS releases
		return true
	}
	return false
}

func isAspectVersionMismatch(aspectRuntime *aspectRuntimeInfo, version string, baseUrl string) bool {
	return aspectRuntime.Version != version || aspectRuntime.BaseUrl != baseUrl
}

func (bazelisk *Bazelisk) getBazelVersion() (string, string, error) {
	// The logic in upstream Bazelisk v1.15.0
	// (https://github.com/bazelbuild/bazelisk/blob/c9081741bc1420d601140a4232b5c48872370fdc/core/core.go#L318)
	// has been updated here to support bootstrapping and reentering a different version and/or tier
	// of Aspect CLI.

	// Gather info on the Aspect CLI version running
	aspectRuntime := &aspectRuntimeInfo{
		Reentrant: len(os.Getenv(aspectReentrantEnv)) != 0,
		Version:   buildinfo.Current().Version(),
		DevBuild:  strings.HasPrefix(buildinfo.Current().Version(), "unknown"),
		BaseUrl:   config.AspectBaseUrl(buildinfo.Current().OpenSource),
	}

	// Get the bazelisk version config from the USE_BAZEL_VERSION and BAZELISK_BASE_URL env vars
	// and/or the .bazeliskrc file
	bazeliskConfig := &bazeliskVersionConfig{
		UseBazelVersion: bazelisk.GetEnvOrConfig(useBazelVersionEnv),
		BazeliskBaseUrl: bazelisk.GetEnvOrConfig(core.BaseURLEnv),
	}

	// If bazelisk is configured to bootstrap the Aspect CLI and the version configured does not
	// match the running version then re-enter that version if we are allowed to re-enter, have not
	// already re-entered
	if isBazeliskAspectBootstrap(bazeliskConfig) {
		// Remove the org from the version string if it is set.
		// For example, "aspect/1.2.3" => "1.2.3".
		s := strings.Split(bazeliskConfig.UseBazelVersion, "/")
		sanitizedUseBazelVersion := s[len(s)-1]
		if bazelisk.allowReenter && !aspectRuntime.Reentrant && isAspectVersionMismatch(aspectRuntime, sanitizedUseBazelVersion, bazeliskConfig.BazeliskBaseUrl) {
			// If bazelisk is configured to bootstrap the CLI and the Aspect CLI config is not then
			// re-enter that version if we are allowed to re-enter and have not already re-entered.
			bazelisk.AspectShouldReenter = true
			return sanitizedUseBazelVersion, bazeliskConfig.BazeliskBaseUrl, nil
		} else {
			// If we decided not to re-enter then scrub the bazelisk configured Aspect CLI version
			// so the logic below falls through to the Bazel version specified in .bazelversion.
			bazeliskConfig = &bazeliskVersionConfig{}
		}
	}

	// If there is bazelisk configured bazel version then we are done
	if len(bazeliskConfig.UseBazelVersion) != 0 {
		return bazeliskConfig.UseBazelVersion, bazeliskConfig.BazeliskBaseUrl, nil
	}

	// Same as upstream bazelisk at this point:
	// https://github.com/bazelbuild/bazelisk/blob/c9081741bc1420d601140a4232b5c48872370fdc/core/core.go#L344

	workspaceRoot := bazelisk.workspaceRoot

	// Load the version from the .bazelversion file if we know the workspace root and it exists
	if len(workspaceRoot) != 0 {
		bazelVersionPath := filepath.Join(workspaceRoot, ".bazelversion")
		if _, err := os.Stat(bazelVersionPath); err == nil {
			f, err := os.Open(bazelVersionPath)
			if err != nil {
				return "", "", fmt.Errorf("could not read %s: %v", bazelVersionPath, err)
			}
			defer f.Close()

			scanner := bufio.NewScanner(f)
			scanner.Scan()
			bazelVersion := scanner.Text()
			if err := scanner.Err(); err != nil {
				return "", "", fmt.Errorf("could not read version from file %s: %v", bazelVersion, err)
			}

			if len(bazelVersion) != 0 {
				return bazelVersion, bazeliskConfig.BazeliskBaseUrl, nil
			}
		}
	}

	// If we still don't have a Bazel version then check for a USE_BAZEL_FALLBACK_VERSION
	fallbackVersionFormat := bazelisk.GetEnvOrConfig("USE_BAZEL_FALLBACK_VERSION")
	fallbackVersionMode, fallbackVersion, hasFallbackVersionMode := strings.Cut(fallbackVersionFormat, ":")
	if !hasFallbackVersionMode {
		fallbackVersionMode, fallbackVersion, hasFallbackVersionMode = "silent", fallbackVersionMode, true
	}
	if len(fallbackVersion) == 0 {
		fallbackVersion = "latest"
	}
	if fallbackVersionMode == "error" {
		return "", "", fmt.Errorf("not allowed to use fallback version %q", fallbackVersion)
	}
	if fallbackVersionMode == "warn" {
		log.Printf("Warning: used fallback version %q\n", fallbackVersion)
		return fallbackVersion, bazeliskConfig.BazeliskBaseUrl, nil
	}
	if fallbackVersionMode == "silent" {
		return fallbackVersion, bazeliskConfig.BazeliskBaseUrl, nil
	}
	return "", "", fmt.Errorf("invalid fallback version format %q (effectively %q)", fallbackVersionFormat, fmt.Sprintf("%s:%s", fallbackVersionMode, fallbackVersion))
}

func downloadBazel(bazelVersionString, baseURL string, bazeliskHome string, repos *core.Repositories) (string, error) {
	bazelFork, bazelVersion, err := parseBazelForkAndVersion(bazelVersionString)
	if err != nil {
		return "", fmt.Errorf("could not parse Bazel fork and version: %v", err)
	}

	resolvedBazelVersion, downloader, err := repos.ResolveVersion(bazeliskHome, bazelFork, bazelVersion)
	if err != nil {
		return "", fmt.Errorf("could not resolve the version '%s' to an actual version number: %v", bazelVersion, err)
	}

	bazelForkOrURL := dirForURL(baseURL)
	if len(bazelForkOrURL) == 0 {
		bazelForkOrURL = bazelFork
	}

	baseDirectory := filepath.Join(bazeliskHome, "downloads", bazelForkOrURL)
	bazelPath, err := downloadBazelIfNecessary(resolvedBazelVersion, baseDirectory, repos, baseURL, downloader)
	return bazelPath, err
}

func downloadBazelIfNecessary(version string, baseDirectory string, repos *core.Repositories, baseURL string, downloader core.DownloadFunc) (string, error) {
	pathSegment, err := platforms.DetermineBazelFilename(version, false)
	if err != nil {
		return "", fmt.Errorf("could not determine path segment to use for Bazel binary: %v", err)
	}

	destDir := filepath.Join(baseDirectory, pathSegment, "bin")
	destFile := "bazel" + platforms.DetermineExecutableFilenameSuffix()

	if baseURL != "" {
		return repos.DownloadFromBaseURL(baseURL, version, destDir, destFile)
	}

	return downloader(destDir, destFile)
}

func (bazelisk *Bazelisk) maybeDelegateToWrapper(bazel string) string {
	if bazelisk.GetEnvOrConfig(skipWrapperEnv) != "" || os.Getenv(aspectReentrantEnv) != "" {
		return bazel
	}

	wrapper := filepath.Join(bazelisk.workspaceRoot, wrapperPath)
	if stat, err := os.Stat(wrapper); err != nil || stat.IsDir() || stat.Mode().Perm()&0111 == 0 {
		return bazel
	}

	return wrapper
}

func (bazelisk *Bazelisk) makeBazelCmd(bazel string, args []string, streams ioutils.Streams, env []string, wd *string, ctx context.Context) *exec.Cmd {
	execPath := bazelisk.maybeDelegateToWrapper(bazel)

	var cmd *exec.Cmd
	if ctx != nil {
		cmd = exec.CommandContext(ctx, execPath, args...)
	} else {
		cmd = exec.Command(execPath, args...)
	}
	cmd.Env = os.Environ()
	if env != nil {
		cmd.Env = append(cmd.Env, env...)
	}
	cmd.Env = append(cmd.Env, aspectReentrantEnv+"=true")
	if execPath != bazel {
		cmd.Env = append(cmd.Env, fmt.Sprintf("%s=%s", bazelReal, bazel))
	}
	if wd != nil {
		cmd.Dir = *wd
	}
	prependDirToPathList(cmd, filepath.Dir(execPath))
	cmd.Stdin = streams.Stdin
	cmd.Stdout = streams.Stdout
	cmd.Stderr = streams.Stderr
	return cmd
}
