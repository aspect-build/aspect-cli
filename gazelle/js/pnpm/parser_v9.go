package gazelle

import "io"

func parsePnpmLockDependenciesV9(yamlReader io.Reader) (WorkspacePackageVersionMap, error) {
	// The top-level lockfile object is the same as v6 for the WorkspacePackageVersionMap requirements
	return parsePnpmLockDependenciesV6(yamlReader)
}
