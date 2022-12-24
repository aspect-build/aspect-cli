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
	"fmt"
	"io"
	"log"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"syscall"

	"github.com/bazelbuild/bazelisk/core"
	"github.com/bazelbuild/bazelisk/httputil"
	"github.com/bazelbuild/bazelisk/platforms"
	"github.com/bazelbuild/bazelisk/versions"
	"github.com/mitchellh/go-homedir"

	"aspect.build/cli/buildinfo"
	"aspect.build/cli/pkg/aspect/root/config"
	"aspect.build/cli/pkg/ioutils"
)

const (
	bazelReal          = "BAZEL_REAL"
	skipWrapperEnv     = "BAZELISK_SKIP_WRAPPER"
	aspectReentrantEnv = "ASPECT_REENTRANT"
	useBazelVersionEnv = "USE_BAZEL_VERSION"
	wrapperPath        = "./tools/bazel"
	rcFileName         = ".bazeliskrc"
)

var (
	fileConfig     map[string]string
	fileConfigOnce sync.Once
)

type Bazelisk struct {
	workspaceRoot string

	// Set to true in getBazelVersion() if this aspect binary is not the user's configured
	// version and should re-enter another aspect binary of a different version
	AspectShouldReenter bool
}

func NewBazelisk(workspaceRoot string) *Bazelisk {
	return &Bazelisk{workspaceRoot: workspaceRoot}
}

// Run runs the main Bazelisk logic for the given arguments and Bazel repositories.
func (bazelisk *Bazelisk) Run(args []string, repos *core.Repositories, streams ioutils.Streams, env []string) (int, error) {
	httputil.UserAgent = bazelisk.getUserAgent()

	bazeliskHome := bazelisk.GetEnvOrConfig("BAZELISK_HOME")
	if len(bazeliskHome) == 0 {
		userCacheDir, err := os.UserCacheDir()
		if err != nil {
			return -1, fmt.Errorf("could not get the user's cache directory: %v", err)
		}

		bazeliskHome = filepath.Join(userCacheDir, "bazelisk")
	}

	err := os.MkdirAll(bazeliskHome, 0755)
	if err != nil {
		return -1, fmt.Errorf("could not create directory %s: %v", bazeliskHome, err)
	}

	bazelVersionString, baseUrl, err := bazelisk.getBazelVersion()
	if err != nil {
		return -1, fmt.Errorf("could not get Bazel version: %v", err)
	}

	bazelPath, err := homedir.Expand(bazelVersionString)
	if err != nil {
		return -1, fmt.Errorf("could not expand home directory in path: %v", err)
	}

	// If the Bazel version is an absolute path to a Bazel binary in the filesystem, we can
	// use it directly. In that case, we don't know which exact version it is, though.
	resolvedBazelVersion := "unknown"

	// If we aren't using a local Bazel binary, we'll have to parse the version string and
	// download the version that the user wants.
	if !filepath.IsAbs(bazelPath) {
		bazelFork, bazelVersion, err := parseBazelForkAndVersion(bazelVersionString)
		if err != nil {
			return -1, fmt.Errorf("could not parse Bazel fork and version: %v", err)
		}

		var downloader core.DownloadFunc
		resolvedBazelVersion, downloader, err = repos.ResolveVersion(bazeliskHome, bazelFork, bazelVersion)
		if err != nil {
			return -1, fmt.Errorf("could not resolve the version '%s' to an actual version number: %v", bazelVersion, err)
		}

		bazelForkOrURL := dirForURL(baseUrl)
		if len(bazelForkOrURL) == 0 {
			bazelForkOrURL = bazelFork
		}

		baseDirectory := filepath.Join(bazeliskHome, "downloads", bazelForkOrURL)
		bazelPath, err = bazelisk.downloadBazel(resolvedBazelVersion, baseDirectory, repos, baseUrl, downloader)
		if err != nil {
			return -1, fmt.Errorf("could not download Bazel: %v", err)
		}
	} else {
		baseDirectory := filepath.Join(bazeliskHome, "local")
		bazelPath, err = linkLocalBazel(baseDirectory, bazelPath)
		if err != nil {
			return -1, fmt.Errorf("cound not link local Bazel: %v", err)
		}
	}

	exitCode, err := bazelisk.runBazel(bazelPath, args, streams, env)
	if err != nil {
		return -1, fmt.Errorf("could not run Bazel: %v", err)
	}
	return exitCode, nil
}

func (bazelisk *Bazelisk) getUserAgent() string {
	agent := bazelisk.GetEnvOrConfig("BAZELISK_USER_AGENT")
	if len(agent) > 0 {
		return agent
	}
	return fmt.Sprintf("Aspect/%s", buildinfo.Current().Version())
}

// GetConfig reads a configuration value from .bazeliskrc.
func (bazelisk *Bazelisk) GetConfig(name string) string {
	fileConfigOnce.Do(bazelisk.loadFileConfig)

	return fileConfig[name]
}

// GetEnvOrConfig reads a configuration value from the environment, but fall back to reading it from .bazeliskrc.
func (bazelisk *Bazelisk) GetEnvOrConfig(name string) string {
	if val := os.Getenv(name); val != "" {
		return val
	}

	fileConfigOnce.Do(bazelisk.loadFileConfig)

	return fileConfig[name]
}

// loadFileConfig locates available .bazeliskrc configuration files, parses them with a precedence order preference,
// and updates a global configuration map with their contents. This routine should be executed exactly once.
func (bazelisk *Bazelisk) loadFileConfig() {
	var rcFilePaths []string

	if userRC, err := locateUserConfigFile(); err == nil {
		rcFilePaths = append(rcFilePaths, userRC)
	}
	if workspaceRC, err := bazelisk.locateWorkspaceConfigFile(); err == nil {
		rcFilePaths = append(rcFilePaths, workspaceRC)
	}

	fileConfig = make(map[string]string)
	for _, rcPath := range rcFilePaths {
		config, err := parseFileConfig(rcPath)
		if err != nil {
			log.Fatal(err)
		}

		for key, value := range config {
			fileConfig[key] = value
		}
	}
}

// locateWorkspaceConfigFile locates a .bazeliskrc file in the current workspace root.
func (bazelisk *Bazelisk) locateWorkspaceConfigFile() (string, error) {
	return filepath.Join(bazelisk.workspaceRoot, rcFileName), nil
}

// locateUserConfigFile locates a .bazeliskrc file in the user's home directory.
func locateUserConfigFile() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, rcFileName), nil
}

// parseFileConfig parses a .bazeliskrc file as a map of key-value configuration values.
func parseFileConfig(rcFilePath string) (map[string]string, error) {
	config := make(map[string]string)

	contents, err := os.ReadFile(rcFilePath)
	if err != nil {
		if os.IsNotExist(err) {
			// Non-critical error.
			return config, nil
		}
		return nil, err
	}

	for _, line := range strings.Split(string(contents), "\n") {
		if strings.HasPrefix(line, "#") {
			// comments
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		if len(parts) < 2 {
			continue
		}
		key := strings.TrimSpace(parts[0])
		config[key] = strings.TrimSpace(parts[1])
	}

	return config, nil
}

type aspectRuntimeInfo struct {
	Reentrant bool
	Version   string
	DevBuild  bool
	BaseUrl   string
}

func (bazelisk *Bazelisk) getBazelVersion() (string, string, error) {
	// The logic in upstream Bazelisk v1.15.0
	// (https://github.com/bazelbuild/bazelisk/blob/c9081741bc1420d601140a4232b5c48872370fdc/core/core.go#L318)
	// has been updated here to support bootstrapping and reentering a different version and/or tier
	// of Aspect CLI -or- ignoring .bazeliskrc config version & base URL in cases where we were bootstrapped
	// bazel Bazelisk or by Aspect or are running a locally install Aspect that matches the version in the
	// Aspect configuration and thus need to determine the Bazel version to run from .bazelversion or from the env.
	//
	// Additional logic to re-enter Aspect or ignore .bazeliskrc config runs in this order:
	//
	// (A) If not bootstrapped by Aspect & an aspect version is configured & not running a
	//     development build, download & re-enter the configured aspect if either the version or base url mismatch with
	//     the aspect running.
	// (B) If bootstrapped by Bazelisk then ignore .bazeliskrc version & base_url (since it specifies
	//     the Aspect version) and use the the env version to determine the Bazel version to download & run.
	// (C) If not bootstrapped by Bazelisk or by Aspect & Aspect version is not configured in Aspect config.yaml
	//     (or by USE_ASPECT_VERSION) & .bazeliskrc is configured to bootstrap Aspect & not running a development
	//     build & no version configured in aspect config, download & re-enter the configured aspect if either the version
	// 		 or base url mismatch with the aspect running.
	// (D) If not bootstrapped by Bazelisk or by Aspect & .bazeliskrc is configured to bootstrap
	//     aspect & we are not re-entering in (C), then ignore .bazeliskrc version & base_url (since it
	//     specifies the Aspect version) and use the the env version to determine the Bazel version to download & run.
	// (E) If not bootstrapped by Bazelisk or by Aspect and .bazeliskrc is NOT configured to bootstrap aspect,
	//     continue with the upstream logic of using GetEnvOrConfig for the bazelVersion and baseUrl.
	//
	// Modified upstream logic to determine what Bazel version to download & use runs in this order:
	//
	// (F) If bazelVersion is set from either .bazeliskrc config or env in the above logic use that version of Bazel
	//     & download from the baseUrl from either .bazeliskrc config or env in the above logic
	// (G) If there is a .bazelversion file with a version specified used that version & download from the baseUrl from
	//     either .bazeliskrc config or env in the above logic
	// (H) If "USE_BAZEL_FALLBACK_VERSION" is set in config or env use that version:
	//     - env var "USE_BAZEL_FALLBACK_VERSION" is set to a fallback version format.
	//     - workspace_root/.bazeliskrc exists -> read contents, in contents:
	//       var "USE_BAZEL_FALLBACK_VERSION" is set to a fallback version format.
	//     - fallback version format "silent:latest"
	aspectRuntime := &aspectRuntimeInfo{
		Reentrant: len(os.Getenv(aspectReentrantEnv)) != 0,
		Version:   buildinfo.Current().Version(),
		DevBuild:  strings.HasPrefix(buildinfo.Current().Version(), "unknown"),
		BaseUrl:   config.AspectBaseUrl(buildinfo.Current().IsAspectPro),
	}

	aspectConfig, err := config.GetVersionConfig()
	if err != nil {
		return "", "", fmt.Errorf("could not get aspect config: %w", err)
	}

	// If an aspect version is configured and we are not already re-entrant that takes precedence.
	if !aspectRuntime.Reentrant && aspectConfig.Configured {
		mismatchVersion := aspectConfig.Version != aspectRuntime.Version
		mismatchBaseUrl := aspectConfig.BaseUrl != aspectRuntime.BaseUrl
		if !aspectRuntime.DevBuild && (mismatchVersion || mismatchBaseUrl) {
			// Re-enter the configured version of aspect if there is a version or base url mismatch
			// and this is not a development build
			bazelisk.AspectShouldReenter = true                    // (A)
			return aspectConfig.Version, aspectConfig.BaseUrl, nil // (A)
		}
	}

	bazelVersion := ""
	baseUrl := ""
	bazeliskReentrant := len(os.Getenv(skipWrapperEnv)) != 0
	if bazeliskReentrant {
		// If we were bootstrapped by bazelisk then we can only use the bazel version &
		// base url from the env or the .bazelversion files below
		bazelVersion = os.Getenv(useBazelVersionEnv) // (B)
		baseUrl = os.Getenv(core.BaseURLEnv)         // (B)
	} else {
		// If we were not bootstrapped by bazelisk we must still check if .bazeliskrc has an aspect bootstrap configured.
		bazeliskBazelVersion := bazelisk.GetConfig(useBazelVersionEnv)
		bazeliskAspectBaseUrl := bazelisk.GetConfig(core.BaseURLEnv)
		bazeliskBootstrapConfigured := len(bazeliskAspectBaseUrl) != 0 && (strings.HasPrefix("aspect/", bazeliskBazelVersion) || strings.Contains(bazeliskAspectBaseUrl, "aspect.build/"))
		if bazeliskBootstrapConfigured {
			splits := strings.Split(bazeliskBazelVersion, "/")
			bazeliskAspectVersion := splits[len(splits)-1]
			mismatchVersion := bazeliskAspectVersion != aspectRuntime.Version
			mismatchBaseUrl := bazeliskAspectBaseUrl != aspectRuntime.BaseUrl
			if !aspectRuntime.Reentrant && !aspectConfig.Configured && !aspectRuntime.DevBuild && (mismatchVersion || mismatchBaseUrl) {
				// Re-enter the configured version of aspect if there is a version or base url mismatch
				// and this is not a development build and no version configured in aspect config (or via USE_ASPECT_VERSION)
				// and we are not already re-entrant.
				bazelisk.AspectShouldReenter = true                      // (C)
				return bazeliskAspectVersion, bazeliskAspectBaseUrl, nil // (C)
			} else {
				// Since bazelisk bootstrap is configured, we can only use the bazel version &
				// base url from the env or the .bazelversion files below
				bazelVersion = os.Getenv(useBazelVersionEnv) // (D)
				baseUrl = os.Getenv(core.BaseURLEnv)         // (D)
			}
		} else {
			// Bazelisk bootstrap is _not_ configured so we can use the .bazeliskrc or env configuration
			// to determine the bazel version to use
			bazelVersion = bazelisk.GetEnvOrConfig(useBazelVersionEnv) // (E)
			baseUrl = bazelisk.GetEnvOrConfig(core.BaseURLEnv)         // (E)
		}
	}

	if len(bazelVersion) != 0 {
		return bazelVersion, baseUrl, nil // (F)
	}

	if len(bazelisk.workspaceRoot) != 0 {
		bazelVersionPath := filepath.Join(bazelisk.workspaceRoot, ".bazelversion")
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
				return bazelVersion, baseUrl, nil // (G)
			}
		}
	}

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
		return fallbackVersion, baseUrl, nil // (H)
	}
	if fallbackVersionMode == "silent" {
		return fallbackVersion, baseUrl, nil // (H)
	}
	return "", "", fmt.Errorf("invalid fallback version format %q (effectively %q)", fallbackVersionFormat, fmt.Sprintf("%s:%s", fallbackVersionMode, fallbackVersion))
}

func parseBazelForkAndVersion(bazelForkAndVersion string) (string, string, error) {
	var bazelFork, bazelVersion string

	versionInfo := strings.Split(bazelForkAndVersion, "/")

	if len(versionInfo) == 1 {
		bazelFork, bazelVersion = versions.BazelUpstream, versionInfo[0]
	} else if len(versionInfo) == 2 {
		bazelFork, bazelVersion = versionInfo[0], versionInfo[1]
	} else {
		return "", "", fmt.Errorf("invalid version \"%s\", could not parse version with more than one slash", bazelForkAndVersion)
	}

	return bazelFork, bazelVersion, nil
}

func (bazelisk *Bazelisk) downloadBazel(version string, baseDirectory string, repos *core.Repositories, baseUrl string, downloader core.DownloadFunc) (string, error) {
	pathSegment, err := platforms.DetermineBazelFilename(version, false)
	if err != nil {
		return "", fmt.Errorf("could not determine path segment to use for Bazel binary: %v", err)
	}

	destFile := "bazel" + platforms.DetermineExecutableFilenameSuffix()
	destinationDir := filepath.Join(baseDirectory, pathSegment, "bin")

	if baseUrl != "" {
		return repos.DownloadFromBaseURL(baseUrl, version, destinationDir, destFile)
	}

	return downloader(destinationDir, destFile)
}

func copyFile(src, dst string, perm os.FileMode) error {
	srcFile, err := os.Open(src)
	if err != nil {
		return err
	}
	defer srcFile.Close()

	dstFile, err := os.OpenFile(dst, os.O_WRONLY|os.O_CREATE, perm)
	if err != nil {
		return err
	}
	defer dstFile.Close()

	_, err = io.Copy(dstFile, srcFile)

	return err
}

func linkLocalBazel(baseDirectory string, bazelPath string) (string, error) {
	normalizedBazelPath := dirForURL(bazelPath)
	destinationDir := filepath.Join(baseDirectory, normalizedBazelPath, "bin")
	err := os.MkdirAll(destinationDir, 0755)
	if err != nil {
		return "", fmt.Errorf("could not create directory %s: %v", destinationDir, err)
	}
	destinationPath := filepath.Join(destinationDir, "bazel"+platforms.DetermineExecutableFilenameSuffix())
	if _, err := os.Stat(destinationPath); err != nil {
		err = os.Symlink(bazelPath, destinationPath)
		// If can't create Symlink, fallback to copy
		if err != nil {
			err = copyFile(bazelPath, destinationPath, 0755)
			if err != nil {
				return "", fmt.Errorf("could not copy file from %s to %s: %v", bazelPath, destinationPath, err)
			}
		}
	}
	return destinationPath, nil
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

func prependDirToPathList(cmd *exec.Cmd, dir string) {
	found := false
	for idx, val := range cmd.Env {
		splits := strings.Split(val, "=")
		if len(splits) != 2 {
			continue
		}
		if strings.EqualFold(splits[0], "PATH") {
			found = true
			cmd.Env[idx] = fmt.Sprintf("PATH=%s%s%s", dir, string(os.PathListSeparator), splits[1])
			break
		}
	}

	if !found {
		cmd.Env = append(cmd.Env, fmt.Sprintf("PATH=%s", dir))
	}
}

func (bazelisk *Bazelisk) makeBazelCmd(bazel string, args []string, streams ioutils.Streams, env []string) *exec.Cmd {
	execPath := bazelisk.maybeDelegateToWrapper(bazel)

	cmd := exec.Command(execPath, args...)
	cmd.Env = os.Environ()
	if env != nil {
		cmd.Env = append(cmd.Env, env...)
	}
	cmd.Env = append(cmd.Env, aspectReentrantEnv+"=true")
	if execPath != bazel {
		cmd.Env = append(cmd.Env, fmt.Sprintf("%s=%s", bazelReal, bazel))
	}
	prependDirToPathList(cmd, filepath.Dir(execPath))
	cmd.Stdin = streams.Stdin
	cmd.Stdout = streams.Stdout
	cmd.Stderr = streams.Stderr
	return cmd
}

func (bazelisk *Bazelisk) runBazel(bazel string, args []string, streams ioutils.Streams, env []string) (int, error) {
	cmd := bazelisk.makeBazelCmd(bazel, args, streams, env)

	err := cmd.Start()
	if err != nil {
		return 1, fmt.Errorf("could not start Bazel: %v", err)
	}

	c := make(chan os.Signal)
	signal.Notify(c, os.Interrupt, syscall.SIGTERM)
	go func() {
		s := <-c

		// Only forward SIGTERM to our child process.
		if s != os.Interrupt {
			cmd.Process.Kill()
		}
	}()

	err = cmd.Wait()
	if err != nil {
		if exitError, ok := err.(*exec.ExitError); ok {
			waitStatus := exitError.Sys().(syscall.WaitStatus)
			return waitStatus.ExitStatus(), nil
		}
		return 1, fmt.Errorf("could not launch Bazel: %v", err)
	}
	return 0, nil
}

// insertArgs will insert newArgs in baseArgs. If baseArgs contains the
// "--" argument, newArgs will be inserted before that. Otherwise, newArgs
// is appended.
func insertArgs(baseArgs []string, newArgs []string) []string {
	var result []string
	inserted := false
	for _, arg := range baseArgs {
		if !inserted && arg == "--" {
			result = append(result, newArgs...)
			inserted = true
		}
		result = append(result, arg)
	}

	if !inserted {
		result = append(result, newArgs...)
	}
	return result
}

func dirForURL(url string) string {
	// Replace all characters that might not be allowed in filenames with "-".
	return regexp.MustCompile("[[:^alnum:]]").ReplaceAllString(url, "-")
}
