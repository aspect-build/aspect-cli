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
	"io"
	"io/fs"
	"net/http"
	"os"
	"path/filepath"
	"runtime"

	"github.com/fatih/color"
)

var (
	faint = color.New(color.Faint)
)

func DownloadPlugin(url string, name string, version string) (string, error) {
	userCacheDir, err := os.UserCacheDir()
	if err != nil {
		return "", fmt.Errorf("could not get the user's cache directory: %v", err)
	}

	pluginsCache := filepath.Join(userCacheDir, "aspect-cli", "plugins", name, version)
	err = os.MkdirAll(pluginsCache, 0755)
	if err != nil {
		return "", fmt.Errorf("could not create directory %s: %v", pluginsCache, err)
	}

	filename, err := determinePluginFilename(name)
	if err != nil {
		return "", fmt.Errorf("unable to determine filename to fetch: %v", err)
	}

	versionedURL := fmt.Sprintf("%s/%s/%s", url, version, filename)

	pluginfile, err := downloadFile(versionedURL, pluginsCache, filename, 0700)
	if err != nil {
		return "", fmt.Errorf("unable to fetch remote plugin from %s: %v", url, err)
	}

	sha256URL := fmt.Sprintf("%s.sha256", versionedURL)
	sha256Filename := fmt.Sprintf("%s.sha256", filename)

	// We don't care if this errors. We have logic to do Trust on first use (TOFU).
	downloadFile(sha256URL, pluginsCache, sha256Filename, 0400)

	return pluginfile, nil
}

// determineBazelFilename returns the correct file name of a local Bazel binary.
// The logic produces the same naming as our /release/release.bzl gives to our aspect-cli binaries.
func determinePluginFilename(pluginName string) (string, error) {
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

	filenameSuffix := ""
	if runtime.GOOS == "windows" {
		filenameSuffix = ".exe"
	}

	return fmt.Sprintf("%s-%s_%s%s", pluginName, osName, machineName, filenameSuffix), nil
}

func downloadFile(originURL, destDir, destFile string, mode fs.FileMode) (string, error) {
	if err := os.MkdirAll(destDir, 0755); err != nil {
		return "", fmt.Errorf("could not create directory %s: %v", destDir, err)
	}
	destinationPath := filepath.Join(destDir, destFile)

	if _, err := os.Stat(destinationPath); err != nil {
		tmpfile, err := os.CreateTemp(destDir, "download")
		if err != nil {
			return "", fmt.Errorf("could not create temporary file: %v", err)
		}
		defer os.Remove(tmpfile.Name())
		defer tmpfile.Close()

		faint.Println("Downloading", originURL)

		resp, err := http.Get(originURL)
		if err != nil {
			return "", fmt.Errorf("HTTP GET %s failed: %v", originURL, err)
		}
		defer resp.Body.Close()

		if resp.StatusCode != 200 {
			return "", fmt.Errorf("HTTP GET %s failed with error %v", originURL, resp.StatusCode)
		}

		if _, err := io.Copy(tmpfile, resp.Body); err != nil {
			return "", fmt.Errorf("could not copy from %s to %s: %v", originURL, tmpfile.Name(), err)
		}

		if err := os.Chmod(tmpfile.Name(), mode); err != nil {
			return "", fmt.Errorf("could not chmod file %s: %v", tmpfile.Name(), err)
		}

		if err := os.Rename(tmpfile.Name(), destinationPath); err != nil {
			return "", fmt.Errorf("could not move %s to %s: %v", tmpfile.Name(), destinationPath, err)
		}
	}

	return destinationPath, nil
}
