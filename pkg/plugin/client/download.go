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

// This file is inspired from https://github.com/bazelbuild/bazelisk/blob/c044e9471ed6a69bad1976dafa312200ae811d5e/platforms/platforms.go#L57

package client

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	"github.com/bazelbuild/bazelisk/httputil"
)

// DetermineExecutableFilenameSuffix returns the extension for binaries on the current operating system.
func DetermineExecutableFilenameSuffix() string {
	filenameSuffix := ""
	if runtime.GOOS == "windows" {
		filenameSuffix = ".exe"
	}
	return filenameSuffix
}

// DetermineBazelFilename returns the correct file name of a local Bazel binary.
// The logic produces the same naming as our /release/release.bzl gives to our aspect-cli binaries.
func DeterminePluginFilename(basename string) (string, error) {
	var machineName string
	switch runtime.GOARCH {
	case "amd64", "arm64":
		machineName = runtime.GOARCH
	default:
		return "", fmt.Errorf("unsupported machine architecture \"%s\", must be arm64 or x86_64", runtime.GOARCH)
	}

	var osName string
	switch runtime.GOOS {
	case "darwin", "linux", "windows":
		osName = runtime.GOOS
	default:
		return "", fmt.Errorf("unsupported operating system \"%s\", must be Linux, macOS or Windows", runtime.GOOS)
	}

	filenameSuffix := DetermineExecutableFilenameSuffix()

	return fmt.Sprintf("%s-%s_%s%s", basename, osName, machineName, filenameSuffix), nil
}

func DownloadPlugin(url string, name string) (string, error) {
	userCacheDir, err := os.UserCacheDir()
	if err != nil {
		return "", fmt.Errorf("could not get the user's cache directory: %v", err)
	}

	pluginsCache := filepath.Join(userCacheDir, "aspect-cli")
	err = os.MkdirAll(pluginsCache, 0755)
	if err != nil {
		return "", fmt.Errorf("could not create directory %s: %v", pluginsCache, err)
	}

	actualUrl, err := DeterminePluginFilename(url)
	if err != nil {
		return "", fmt.Errorf("unable to determine filename to fetch: %v", err)
	}

	pluginfile, err := httputil.DownloadBinary(actualUrl, pluginsCache, name)
	if err != nil {
		return "", fmt.Errorf("unable to fetch remote plugin from %s: %v", url, err)
	}
	return pluginfile, nil
}
