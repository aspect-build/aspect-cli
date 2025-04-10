// VENDORED https://github.com/bazelbuild/bazelisk/blob/v1.25.0/core/core.go
//
// Minor changes made to align with the ./bazelisk.go API, which is a mix of custom code
// and vendored code significantly different then the origin bazelisk/core/core.go.
//
// DO NOT MODIFY ... without diffing with the upstream file.

// Package core contains the core Bazelisk logic, as well as abstractions for Bazel repositories.
package bazel

// TODO: split this file into multiple smaller ones in dedicated packages (e.g. execution, incompatible, ...).

import (
	"bufio"
	"context"
	"crypto/sha256"
	"fmt"
	"io"
	"log"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"regexp"
	"runtime"
	"strings"
	"syscall"

	"github.com/aspect-build/aspect-cli/buildinfo"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/bazelbuild/bazelisk/config"
	"github.com/bazelbuild/bazelisk/core"
	"github.com/bazelbuild/bazelisk/platforms"
	"github.com/bazelbuild/bazelisk/versions"
	"github.com/mitchellh/go-homedir"
)

const (
	bazelReal               = "BAZEL_REAL"
	skipWrapperEnv          = "ASPECT_REENTRANT"
	bazeliskEnv             = "BAZELISK"
	defaultWrapperDirectory = "./tools"
	defaultWrapperName      = "bazel"
	maxDirLength            = 255
)

// BazelInstallation provides a summary of a single install of `bazel`
type BazelInstallation struct {
	Version string
	Path    string
}

// GetBazelInstallation provides a mechanism to find the `bazel` binary to execute, as well as its version
func (bazelisk *Bazelisk) GetBazelInstallation(repos *core.Repositories, config config.Config) (*BazelInstallation, error) {
	bazeliskHome, err := getBazeliskHome(config)
	if err != nil {
		return nil, fmt.Errorf("could not determine Bazelisk home directory: %v", err)
	}

	err = os.MkdirAll(bazeliskHome, 0755)
	if err != nil {
		return nil, fmt.Errorf("could not create directory %s: %v", bazeliskHome, err)
	}

	bazelVersionString, baseUrl, err := bazelisk.getBazelVersionAndUrl()
	if err != nil {
		return nil, fmt.Errorf("could not get Bazel version: %v", err)
	}

	if bazelisk.AspectShouldReenter {
		// Work-around for re-entering older versions of the Aspect CLI that didn't handle
		// env boostrap correctly.
		scrubEnvOfBazeliskAspectBootstrap()
	}

	bazelPath, err := homedir.Expand(bazelVersionString)
	if err != nil {
		return nil, fmt.Errorf("could not expand home directory in path: %v", err)
	}

	var resolvedVersion string

	// If we aren't using a local Bazel binary, we'll have to parse the version string and
	// download the version that the user wants.
	if !filepath.IsAbs(bazelPath) {
		resolvedVersion = bazelVersionString
		bazelPath, err = downloadBazel(bazelVersionString, baseUrl, bazeliskHome, repos, config)
		if err != nil {
			return nil, fmt.Errorf("could not download Bazel: %v", err)
		}
	} else {
		// If the Bazel version is an absolute path to a Bazel binary in the filesystem, we can
		// use it directly. In that case, we don't know which exact version it is, though.
		resolvedVersion = "unknown"
		baseDirectory := filepath.Join(bazeliskHome, "local")
		bazelPath, err = linkLocalBazel(baseDirectory, bazelPath)
		if err != nil {
			return nil, fmt.Errorf("could not link local Bazel: %v", err)
		}
	}

	return &BazelInstallation{
			Version: resolvedVersion,
			Path:    bazelPath,
		},
		nil
}

// getBazeliskHome returns the path to the Bazelisk home directory.
func getBazeliskHome(config config.Config) (string, error) {
	bazeliskHome := config.Get("BAZELISK_HOME_" + strings.ToUpper(runtime.GOOS))
	if len(bazeliskHome) == 0 {
		bazeliskHome = config.Get("BAZELISK_HOME")
	}

	if len(bazeliskHome) == 0 {
		userCacheDir, err := os.UserCacheDir()
		if err != nil {
			return "", fmt.Errorf("could not get the user's cache directory: %v", err)
		}

		bazeliskHome = filepath.Join(userCacheDir, "bazelisk")
	} else {
		// If a custom BAZELISK_HOME is set, handle tilde and var expansion
		// before creating the Bazelisk home directory.
		var err error

		bazeliskHome, err = homedir.Expand(bazeliskHome)
		if err != nil {
			return "", fmt.Errorf("could not expand home directory in path: %v", err)
		}

		bazeliskHome = os.ExpandEnv(bazeliskHome)
	}

	return bazeliskHome, nil
}

// MODIFIED: for Aspect branding and buildinfo
func getUserAgent(config config.Config) string {
	agent := config.Get("BAZELISK_USER_AGENT")
	if len(agent) > 0 {
		return agent
	}
	return fmt.Sprintf("Aspect/%s", buildinfo.Current().Version())
}

// GetBazelVersion returns the Bazel version that should be used.
// MODIFIED: to use *Bazelisk instance
func (bazelisk *Bazelisk) GetBazelVersion(config config.Config) (string, error) {
	// Check in this order:
	// - env var "USE_BAZEL_VERSION" is set to a specific version.
	// - workspace_root/.bazeliskrc exists -> read contents, in contents:
	//   var "USE_BAZEL_VERSION" is set to a specific version.
	// - env var "USE_NIGHTLY_BAZEL" or "USE_BAZEL_NIGHTLY" is set -> latest
	//   nightly. (TODO)
	// - env var "USE_CANARY_BAZEL" or "USE_BAZEL_CANARY" is set -> latest
	//   rc. (TODO)
	// - the file workspace_root/tools/bazel exists -> that version. (TODO)
	// - workspace_root/.bazelversion exists -> read contents, that version.
	// - workspace_root/WORKSPACE contains a version -> that version. (TODO)
	// - env var "USE_BAZEL_FALLBACK_VERSION" is set to a fallback version format.
	// - workspace_root/.bazeliskrc exists -> read contents, in contents:
	//   var "USE_BAZEL_FALLBACK_VERSION" is set to a fallback version format.
	// - fallback version format "silent:latest"

	// MODIFIED: "USE_BAZEL_VERSION" handled in parent
	// bazelVersion := config.Get("USE_BAZEL_VERSION")
	// if len(bazelVersion) != 0 {
	// 	return bazelVersion, nil
	// }

	workspaceRoot := bazelisk.workspaceRoot
	if len(workspaceRoot) != 0 {
		bazelVersionPath := filepath.Join(workspaceRoot, ".bazelversion")
		if _, err := os.Stat(bazelVersionPath); err == nil {
			f, err := os.Open(bazelVersionPath)
			if err != nil {
				return "", fmt.Errorf("could not read %s: %v", bazelVersionPath, err)
			}
			defer f.Close()

			scanner := bufio.NewScanner(f)
			scanner.Scan()
			bazelVersion := scanner.Text()
			if err := scanner.Err(); err != nil {
				return "", fmt.Errorf("could not read version from file %s: %v", bazelVersion, err)
			}

			if len(bazelVersion) != 0 {
				return bazelVersion, nil
			}
		}
	}

	fallbackVersionFormat := config.Get("USE_BAZEL_FALLBACK_VERSION")
	fallbackVersionMode, fallbackVersion, hasFallbackVersionMode := strings.Cut(fallbackVersionFormat, ":")
	if !hasFallbackVersionMode {
		fallbackVersionMode, fallbackVersion, hasFallbackVersionMode = "silent", fallbackVersionMode, true
	}
	if len(fallbackVersion) == 0 {
		fallbackVersion = "latest"
	}
	if fallbackVersionMode == "error" {
		return "", fmt.Errorf("not allowed to use fallback version %q", fallbackVersion)
	}
	if fallbackVersionMode == "warn" {
		log.Printf("Warning: used fallback version %q\n", fallbackVersion)
		return fallbackVersion, nil
	}
	if fallbackVersionMode == "silent" {
		return fallbackVersion, nil
	}
	return "", fmt.Errorf("invalid fallback version format %q (effectively %q)", fallbackVersionFormat, fmt.Sprintf("%s:%s", fallbackVersionMode, fallbackVersion))
}

func parseBazelForkAndVersion(bazelForkAndVersion string) (string, string, error) {
	var bazelFork, bazelVersion string

	versionInfo := strings.Split(bazelForkAndVersion, "/")

	if len(versionInfo) == 1 {
		bazelFork, bazelVersion = versions.BazelUpstream, versionInfo[0]
	} else if len(versionInfo) == 2 {
		bazelFork, bazelVersion = versionInfo[0], versionInfo[1]
	} else {
		return "", "", fmt.Errorf("invalid version %q, could not parse version with more than one slash", bazelForkAndVersion)
	}

	return bazelFork, bazelVersion, nil
}

// MODIFIED: to replace BaseURLEnv env lookup with baseUrl param
func downloadBazel(bazelVersionString, baseURL string, bazeliskHome string, repos *core.Repositories, config config.Config) (string, error) {
	bazelFork, bazelVersion, err := parseBazelForkAndVersion(bazelVersionString)
	if err != nil {
		return "", fmt.Errorf("could not parse Bazel fork and version: %v", err)
	}

	resolvedBazelVersion, downloader, err := repos.ResolveVersion(bazeliskHome, bazelFork, bazelVersion, config)
	if err != nil {
		return "", fmt.Errorf("could not resolve the version '%s' to an actual version number: %v", bazelVersion, err)
	}

	bazelForkOrURL := dirForURL(baseURL)
	if len(bazelForkOrURL) == 0 {
		bazelForkOrURL = bazelFork
	}

	bazelPath, err := downloadBazelIfNecessary(resolvedBazelVersion, bazeliskHome, bazelForkOrURL, repos, config, baseURL, downloader)
	return bazelPath, err
}

// MODIFIED: to replace BaseURLEnv env lookup with baseUrl param
func downloadBazelIfNecessary(version string, bazeliskHome string, bazelForkOrURLDirName string, repos *core.Repositories, config config.Config, baseURL string, downloader core.DownloadFunc) (string, error) {
	pathSegment, err := platforms.DetermineBazelFilename(version, false, config)
	if err != nil {
		return "", fmt.Errorf("could not determine path segment to use for Bazel binary: %v", err)
	}

	baseDirectory := filepath.Join(bazeliskHome, "downloads", bazelForkOrURLDirName)
	destDir := filepath.Join(baseDirectory, pathSegment, "bin")
	// MODIFIED: remove `BAZELISK_VERIFY_SHA256`
	destFile := "bazel" + platforms.DetermineExecutableFilenameSuffix()

	// MODIFIED: remove all custom URL/downloading, remove expectedSha256 verification
	if baseURL != "" {
		return repos.DownloadFromBaseURL(baseURL, version, destDir, destFile, config)
	}

	return downloader(destDir, destFile)
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

// MODIFIED: to use *Bazelisk instance
func (bazelisk *Bazelisk) maybeDelegateToWrapperFromDir(bazel string, wd string, config config.Config) string {
	if config.Get(skipWrapperEnv) != "" {
		return bazel
	}

	wrapperPath := config.Get("BAZELISK_WRAPPER_DIRECTORY")
	if wrapperPath == "" {
		wrapperPath = filepath.Join(defaultWrapperDirectory, defaultWrapperName)
	} else {
		wrapperPath = filepath.Join(wrapperPath, defaultWrapperName)
	}

	root := bazelisk.workspaceRoot
	wrapper := filepath.Join(root, wrapperPath)
	if stat, err := os.Stat(wrapper); err == nil && !stat.Mode().IsDir() && stat.Mode().Perm()&0111 != 0 {
		return wrapper
	}

	if runtime.GOOS == "windows" {
		powershellWrapper := filepath.Join(root, wrapperPath+".ps1")
		if stat, err := os.Stat(powershellWrapper); err == nil && !stat.Mode().IsDir() {
			return powershellWrapper
		}

		batchWrapper := filepath.Join(root, wrapperPath+".bat")
		if stat, err := os.Stat(batchWrapper); err == nil && !stat.Mode().IsDir() {
			return batchWrapper
		}
	}

	return bazel
}

// MODIFIED: to use *Bazelisk instance
func (bazelisk *Bazelisk) maybeDelegateToWrapper(bazel string, config config.Config) string {
	wd, err := os.Getwd()
	if err != nil {
		return bazel
	}

	return bazelisk.maybeDelegateToWrapperFromDir(bazel, wd, config)
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

// MODIFIED: to use *Bazelisk instance, ioutils.Streams, maintain legacy env[]
func (bazelisk *Bazelisk) makeBazelCmd(bazel string, args []string, streams ioutils.Streams, env []string, config config.Config, wd *string, ctx context.Context) *exec.Cmd {
	execPath := bazelisk.maybeDelegateToWrapper(bazel, config)

	// MODIFIED: to support ctx
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
	cmd.Env = append(cmd.Env, skipWrapperEnv+"=true")
	if execPath != bazel {
		cmd.Env = append(cmd.Env, fmt.Sprintf("%s=%s", bazelReal, bazel))
	}
	selfPath, err := os.Executable()
	if err != nil {
		cmd.Env = append(cmd.Env, bazeliskEnv+"="+selfPath)
	}
	// MODIFIED: to support cmd.Dir
	if wd != nil {
		cmd.Dir = *wd
	}
	prependDirToPathList(cmd, filepath.Dir(execPath))
	// MODIFIED: to support streams.*
	cmd.Stdin = streams.Stdin
	cmd.Stdout = streams.Stdout
	cmd.Stderr = streams.Stderr
	return cmd
}

// MODIFIED: to use *Bazelisk instance, ioutils.Streams, maintain legacy env[]
func (bazelisk *Bazelisk) runBazel(bazel string, args []string, streams ioutils.Streams, env []string, config config.Config, wd *string) (int, error) {
	cmd := bazelisk.makeBazelCmd(bazel, args, streams, env, config, wd, nil)

	err := cmd.Start()
	if err != nil {
		return 1, fmt.Errorf("could not start Bazel: %v", err)
	}

	c := make(chan os.Signal, 1)
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

func dirForURL(url string) string {
	// Replace all characters that might not be allowed in filenames with "-".
	dir := regexp.MustCompile("[[:^alnum:]]").ReplaceAllString(url, "-")
	// Work around length limit on some systems by truncating and then appending
	// a sha256 hash of the URL.
	if len(dir) > maxDirLength {
		suffix := fmt.Sprintf("...%x", sha256.Sum256([]byte(url)))
		dir = dir[:maxDirLength-len(suffix)] + suffix
	}
	return dir
}
