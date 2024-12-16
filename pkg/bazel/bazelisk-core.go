// VENDORED https://github.com/bazelbuild/bazelisk/blob/v1.17.0/core/core.go
//
// Minor changes made to align with the ./bazelisk.go API, which is a mix of custom code
// and vendored code significantly different then the origin bazelisk/core/core.go.
//
// DO NOT MODIFY ... without diffing with the upstream file.

// Package core contains the core Bazelisk logic, as well as abstractions for Bazel repositories.
package bazel

// TODO: split this file into multiple smaller ones in dedicated packages (e.g. execution, incompatible, ...).

import (
	"crypto/sha256"
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

	"github.com/aspect-build/aspect-cli/buildinfo"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/bazelbuild/bazelisk/platforms"
	"github.com/bazelbuild/bazelisk/versions"
)

const (
	bazelReal      = "BAZEL_REAL"
	skipWrapperEnv = "BAZELISK_SKIP_WRAPPER"
	wrapperPath    = "./tools/bazel"
	rcFileName     = ".bazeliskrc"
	maxDirLength   = 255
)

var (
	fileConfig     map[string]string
	fileConfigOnce sync.Once
)

func (bazelisk *Bazelisk) getUserAgent() string {
	agent := bazelisk.GetEnvOrConfig("BAZELISK_USER_AGENT")
	if len(agent) > 0 {
		return agent
	}
	return fmt.Sprintf("Aspect/%s", buildinfo.Current().Version())
}

// GetEnvOrConfig reads a configuration value from the environment, but fall back to reading it from .bazeliskrc.
func (bazelisk *Bazelisk) GetEnvOrConfig(name string) string {
	if val, found := os.LookupEnv(name); found {
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

// MODIFIED to use the Bazelisk struct
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

// isValidWorkspace returns true iff the supplied path is the workspace root, defined by the presence of
// a file named WORKSPACE or WORKSPACE.bazel
// see https://github.com/bazelbuild/bazel/blob/8346ea4cfdd9fbd170d51a528fee26f912dad2d5/src/main/cpp/workspace_layout.cc#L37
func isValidWorkspace(path string) bool {
	info, err := os.Stat(path)
	if err != nil {
		return false
	}

	return !info.IsDir()
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

func (bazelisk *Bazelisk) runBazel(bazel string, args []string, streams ioutils.Streams, env []string, wd *string) (int, error) {
	cmd := bazelisk.makeBazelCmd(bazel, args, streams, env, wd)

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
