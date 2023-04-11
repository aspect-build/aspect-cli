package gazelle

import (
	"fmt"

	"gopkg.in/yaml.v3"
)

func parsePnpmLockDependenciesV6(yamlFileContent []byte) (WorkspacePackageVersionMap, error) {
	lockfile := PnpmLockfileV6{}
	unmarshalErr := yaml.Unmarshal(yamlFileContent, &lockfile)
	if unmarshalErr != nil {
		return nil, fmt.Errorf("parse error: %v", unmarshalErr)
	}

	result := make(WorkspacePackageVersionMap)

	// Pnpm workspace lockfiles contain a list of projects under Importers, including
	// the root project as the "." importer.
	if lockfile.Importers != nil {
		for pkg, pkgDeps := range lockfile.Importers {
			result[pkg] = mergeDependenciesV6(pkgDeps.Dependencies, pkgDeps.DevDependencies, pkgDeps.PeerDependencies, pkgDeps.OptionalDependencies)
		}
	} else {
		// Non-workspace lockfiles have one set of dependencies at the root
		result["."] = mergeDependenciesV6(lockfile.Dependencies, lockfile.DevDependencies, lockfile.PeerDependencies, lockfile.OptionalDependencies)
	}

	return result, nil
}

/*
	  Example v6 pnpm-lock.yaml without workspaces

	  ```
		lockfileVersion: '6.0'

		dependencies:
		'@aspect-test/c':
			specifier: ^2.0.2
			version: 2.0.2

		devDependencies:
		jquery:
			specifier: 3.6.1
			version: 3.6.1

		packages:

		/@aspect-test/c@2.0.2:
				...
			...
		...
	  ```

	  or with pnpm-workspace.yaml:

	  ```
		lockfileVersion: '6.0'

		importers:

		.:
			dependencies:
			'@aspect-test/c':
				specifier: ^2.0.2
				version: 2.0.2
			devDependencies:
			jquery:
				specifier: 3.6.1
				version: 3.6.1

		packages:

		/@aspect-test/c@2.0.2:

				...
			...
		...
	  ```
*/

type packageInfoV6 = map[string]struct {
	Version string
}

type PnpmLockfileV6 struct {
	Dependencies         packageInfoV6
	DevDependencies      packageInfoV6 `yaml:"devDependencies"`
	PeerDependencies     packageInfoV6 `yaml:"peerDependencies"`
	OptionalDependencies packageInfoV6 `yaml:"optionalDependencies"`

	Importers map[string]struct {
		Dependencies         packageInfoV6
		DevDependencies      packageInfoV6 `yaml:"devDependencies"`
		PeerDependencies     packageInfoV6 `yaml:"peerDependencies"`
		OptionalDependencies packageInfoV6 `yaml:"optionalDependencies"`
	}
}

func mergeDependenciesV6(d ...packageInfoV6) map[string]string {
	result := make(map[string]string)

	for _, m := range d {
		for k, v := range m {
			result[k] = v.Version
		}
	}

	return result
}
