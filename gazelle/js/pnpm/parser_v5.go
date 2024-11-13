package gazelle

import (
	"fmt"
	"io"

	"gopkg.in/yaml.v3"
)

func parsePnpmLockDependenciesV5(yamlReader io.Reader) (WorkspacePackageVersionMap, error) {
	lockfile := PnpmLockfileV5{}
	unmarshalErr := yaml.NewDecoder(yamlReader).Decode(&lockfile)
	if unmarshalErr == io.EOF {
		return nil, nil
	}
	if unmarshalErr != nil {
		return nil, fmt.Errorf("parse error: %v", unmarshalErr)
	}

	result := make(WorkspacePackageVersionMap)

	// Pnpm workspace lockfiles contain a list of projects under Importers, including
	// the root project as the "." importer.
	if lockfile.Importers != nil {
		for pkg, pkgDeps := range lockfile.Importers {
			result[pkg] = mergeDependenciesV5(pkgDeps.Dependencies, pkgDeps.DevDependencies, pkgDeps.PeerDependencies, pkgDeps.OptionalDependencies)
		}
	} else {
		// Non-workspace lockfiles have one set of dependencies at the root
		result["."] = mergeDependenciesV5(lockfile.Dependencies, lockfile.DevDependencies, lockfile.PeerDependencies, lockfile.OptionalDependencies)
	}

	return result, nil
}

/*
	  Example v5 pnpm-lock.yaml without workspaces

	  ```
		lockfileVersion: 5.4
		specifiers:
			'@aspect-test/c': ^2.0.2
	  		jquery: 3.6.1
		dependencies:
			'@aspect-test/c': 2.0.2
		devDependencies:
			jquery: 3.6.1
		packages:
			/@aspect-test/c/2.0.2:
				...
			...
		...
	  ```

	  or with pnpm-workspaces.yaml:

	  ```
		lockfileVersion: 5.4
		importers:
			.:
				specifiers:
					'@aspect-test/a': ^2.0.2
				dependencies:
					'@aspect-test/a': ^2.0.2
			gazelle/ts/tests/simple_json_import:
				specifiers: {}
			infrastructure/cdn:
				specifiers:
					'@aspect-test/c': ^2.0.2
				dependencies:
					'@aspect-test/c': ^2.0.2
		packages:
			/@aspect-test/c/2.0.2:
				...
			...
		...
	  ```
*/

type packageInfoV5 = map[string]string

type PnpmLockfileV5 struct {
	Dependencies         packageInfoV5
	DevDependencies      packageInfoV5 `yaml:"devDependencies"`
	PeerDependencies     packageInfoV5 `yaml:"peerDependencies"`
	OptionalDependencies packageInfoV5 `yaml:"optionalDependencies"`

	Importers map[string]struct {
		Dependencies         packageInfoV5
		DevDependencies      packageInfoV5 `yaml:"devDependencies"`
		PeerDependencies     packageInfoV5 `yaml:"peerDependencies"`
		OptionalDependencies packageInfoV5 `yaml:"optionalDependencies"`
	}
}

func mergeDependenciesV5(d ...packageInfoV5) map[string]string {
	result := make(map[string]string)

	for _, m := range d {
		for k, v := range m {
			result[k] = v
		}
	}

	return result
}
