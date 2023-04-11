package gazelle

import (
	"fmt"
	"log"
	"os"
	"regexp"

	semver "github.com/Masterminds/semver/v3"
)

type WorkspacePackageVersionMap map[string]map[string]string

/* Parse a lockfile and return a map of workspace projects to a map of dependency name to version.
 */
func ParsePnpmLockFileDependencies(lockfilePath string) WorkspacePackageVersionMap {
	yamlFileContent, readErr := os.ReadFile(lockfilePath)
	if readErr != nil {
		log.Fatalf("failed to read lockfile '%s': %s", lockfilePath, readErr.Error())
	}

	deps, err := parsePnpmLockDependencies(yamlFileContent)
	if err != nil {
		log.Fatalf("pnpm parse - %v\n", err)
	}
	return deps
}

var lockVersionRegex = regexp.MustCompile(`^\s*lockfileVersion: '?(?P<Version>\d\.\d)'?`)

func parsePnpmLockVersion(yamlFileContent []byte) (string, error) {
	match := lockVersionRegex.FindStringSubmatch(string(yamlFileContent))

	if len(match) != 2 {
		return "", fmt.Errorf("failed to find lockfile version in: %q", string(yamlFileContent))
	}

	return match[1], nil
}

func parsePnpmLockDependencies(yamlFileContent []byte) (WorkspacePackageVersionMap, error) {
	if len(yamlFileContent) == 0 {
		return WorkspacePackageVersionMap{}, nil
	}

	versionStr, versionErr := parsePnpmLockVersion(yamlFileContent)
	if versionErr != nil {
		return nil, versionErr
	}

	version, versionErr := semver.NewVersion(versionStr)
	if versionErr != nil {
		return nil, fmt.Errorf("failed to parse semver %q: %v", versionStr, versionErr)
	}

	if version.Major() == 5 {
		return parsePnpmLockDependenciesV5(yamlFileContent)
	} else if version.Major() == 6 {
		return parsePnpmLockDependenciesV6(yamlFileContent)
	}

	return nil, fmt.Errorf("unsupported version: %v", versionStr)
}
