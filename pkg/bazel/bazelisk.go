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
	"fmt"
	"os"
	"strings"

	bazeliskConfig "github.com/bazelbuild/bazelisk/config"
	"github.com/bazelbuild/bazelisk/core"
	"github.com/bazelbuild/bazelisk/httputil"

	"github.com/aspect-build/aspect-cli/buildinfo"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/config"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

const (
	aspectReentrantEnv = "ASPECT_REENTRANT"
	useBazelVersionEnv = "USE_BAZEL_VERSION"
)

type Bazelisk struct {
	workspaceRoot string

	allowReenter bool

	config bazeliskConfig.Config

	// Set to true in getBazelVersionAndUrl() if this aspect binary is not the user's configured
	// version and should re-enter another aspect binary of a different version
	AspectShouldReenter bool
}

func NewBazelisk(workspaceRoot string, allowReenter bool) *Bazelisk {
	return &Bazelisk{
		config:        core.MakeDefaultConfig(),
		workspaceRoot: workspaceRoot,
		allowReenter:  allowReenter,
	}
}

// Run runs the main Bazelisk logic for the given arguments and Bazel repositories.
func (bazelisk *Bazelisk) Run(args []string, repos *core.Repositories, streams ioutils.Streams, env []string, config bazeliskConfig.Config, wd *string) error {
	httputil.UserAgent = getUserAgent(config)

	bazelInstallation, err := bazelisk.GetBazelInstallation(repos, config)
	if err != nil {
		return fmt.Errorf("could not get path to Bazel: %v", err)
	}

	exitCode, err := bazelisk.runBazel(bazelInstallation.Path, args, streams, env, config, wd)
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

func (bazelisk *Bazelisk) getBazelVersionAndUrl() (string, string, error) {
	// The logic wraps the Bazelisk GetBazelVersion() to add support for bootstrapping
	// and reentering a different version and/or tier of Aspect CLI.

	// Gather info on the Aspect CLI version running
	aspectRuntime := &aspectRuntimeInfo{
		Reentrant: os.Getenv(aspectReentrantEnv) != "",
		Version:   buildinfo.Current().Version(),
		DevBuild:  strings.HasPrefix(buildinfo.Current().Version(), "unknown"),
		BaseUrl:   config.AspectBaseUrl(),
	}

	// Get the bazelisk version config from the USE_BAZEL_VERSION and BAZELISK_BASE_URL env vars
	// and/or the .bazeliskrc file
	bazeliskConfig := &bazeliskVersionConfig{
		UseBazelVersion: bazelisk.config.Get(useBazelVersionEnv),
		BazeliskBaseUrl: bazelisk.config.Get(core.BaseURLEnv),
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
	v, err := bazelisk.GetBazelVersion(bazelisk.config)
	if err != nil {
		return "", "", err
	}
	return v, bazeliskConfig.BazeliskBaseUrl, err
}
