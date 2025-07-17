package gazelle

import (
	"bufio"
	"bytes"
	"encoding/gob"
	"fmt"
	"io"
	"regexp"

	semver "github.com/Masterminds/semver/v3"
)

type WorkspacePackageVersionMap map[string]map[string]string

func init() {
	gob.Register(WorkspacePackageVersionMap{})
}

/* Parse a lockfile and return a map of workspace projects to a map of dependency name to version.
 */
func ParsePnpmLockFileDependencies(lockfileContent []byte) (WorkspacePackageVersionMap, error) {
	return parsePnpmLockDependencies(bytes.NewReader(lockfileContent))
}

var lockVersionRegex = regexp.MustCompile(`^\s*lockfileVersion: '?(?P<Version>\d\.\d)'?`)

func parsePnpmLockVersion(yamlFileReader *bufio.Reader) (string, error) {
	versionBytes, isShort, err := yamlFileReader.ReadLine()

	if isShort {
		return "", fmt.Errorf("failed to read lockfile version, line too long: '%s...'", string(versionBytes))
	}
	if err == io.EOF {
		return "", nil
	}
	if err != nil {
		return "", fmt.Errorf("failed to read lockfile version: %v", err)
	}

	match := lockVersionRegex.FindSubmatch(versionBytes)

	if len(match) != 2 {
		return "", fmt.Errorf("failed to find lockfile version in: %q", string(versionBytes))
	}

	return string(match[1]), nil
}

func parsePnpmLockDependencies(yamlFileReader io.Reader) (WorkspacePackageVersionMap, error) {
	yamlReader := bufio.NewReader(yamlFileReader)

	versionStr, versionErr := parsePnpmLockVersion(yamlReader)
	if versionStr == "" || versionErr != nil {
		return nil, versionErr
	}

	version, versionErr := semver.NewVersion(versionStr)
	if versionErr != nil {
		return nil, fmt.Errorf("failed to parse semver %q: %v", versionStr, versionErr)
	}

	if version.Major() == 5 {
		return parsePnpmLockDependenciesV5(yamlReader)
	} else if version.Major() == 6 {
		return parsePnpmLockDependenciesV6(yamlReader)
	} else if version.Major() == 9 {
		return parsePnpmLockDependenciesV9(yamlReader)
	}

	return nil, fmt.Errorf("unsupported version: %v", versionStr)
}
