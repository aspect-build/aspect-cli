package gazelle

func parsePnpmLockDependenciesV9(yamlFileContent []byte) (WorkspacePackageVersionMap, error) {
	// The top-level lockfile object is the same as v6 for the WorkspacePackageVersionMap requirements
	return parsePnpmLockDependenciesV6(yamlFileContent)
}
